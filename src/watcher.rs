//! The light watch at work: a check every minute against matterjs-server, the reconnections,
//! pings and messages it calls for, and two weeks of history. [`crate::watch`] decides; this
//! module only carries the decisions out.

use std::{collections::BTreeMap, fmt::Write as _, time::Duration};

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::{
    ApiError, ApiResult, AppState, Db, Device, UNPOISONED, devices_of, ledger,
    link::{Link, OPEN_TIMEOUT, Outcome, next_id},
    match_ledger, matter, place,
    rejoin::{Leaves, Pursuit, Reason},
    watch::{Cause, Change, Key, Left, LeftBy, Notice, Outage, Seen},
};

/// One check's reading of every node and of the lights due a read.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// A light's answer to a read of its On/Off.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// matterjs-server's answer to `interview_node`, a full read of the node over a new session.
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(40);
/// matterjs-server's answer to `ping_node`: an mDNS lookup of up to 3 s and pings of up to 10 s.
const PING_TIMEOUT: Duration = Duration::from_secs(20);
/// The history is kept this many seconds.
const KEPT: i64 = 14 * 86_400;
/// Notices kept to send again while the destination fails; the oldest go first.
const MAX_UNSENT: usize = 20;
/// A check this close before a moment answers for it, in seconds: a minute and its slack.
const CHECK_COVERS: i64 = 90;
/// The connection listening for removed nodes is opened again after this long without a message.
const LISTEN_IDLE: Duration = Duration::from_mins(10);
/// The connection listening for removed nodes is opened again this long after it fails.
const LISTEN_AGAIN: Duration = Duration::from_secs(10);
/// Removed nodes kept to tell when a light left.
const MAX_REMOVED: usize = 100;

/// Version 12 keeps two weeks of the light watch: every stretch a light or matterjs-server was
/// out of reach, and which lights each check found connected.
pub(crate) fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "BEGIN;
        CREATE TABLE IF NOT EXISTS light_outages (
            key TEXT NOT NULL,
            node_id INTEGER,
            endpoint INTEGER,
            name TEXT NOT NULL,
            network TEXT,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            found_by TEXT NOT NULL,
            reconnects INTEGER NOT NULL DEFAULT 0,
            reconnect_result TEXT,
            reachable INTEGER,
            notified INTEGER NOT NULL DEFAULT 0,
            advised INTEGER NOT NULL DEFAULT 0,
            UNIQUE (key, started_at)
        );
        CREATE INDEX IF NOT EXISTS light_outages_started_at ON light_outages (started_at);
        CREATE TABLE IF NOT EXISTS light_checks (
            at INTEGER PRIMARY KEY,
            server INTEGER NOT NULL,
            lights TEXT NOT NULL
        );
        PRAGMA user_version = 12;
        COMMIT;",
    )
}

/// Version 13 keeps what is known of a light that leaves the fabric: its ledger row, when and
/// how it left, its labels, serial number, last Software Version and Thread parent, and whether
/// the user was told; and what each ledger device last was while matterjs-server served it.
pub(crate) fn migrate_left(db: &Connection) -> rusqlite::Result<()> {
    let added = missing_columns(
        db,
        &[
            ("device_id", "INTEGER"),
            ("left_at", "INTEGER"),
            ("left_by", "TEXT"),
            ("labels", "TEXT NOT NULL DEFAULT '[]'"),
            ("serial_number", "TEXT"),
            ("software_version", "TEXT"),
            ("thread_parent", "TEXT"),
            ("left_told", "INTEGER NOT NULL DEFAULT 0"),
        ],
    )?;
    db.execute_batch(&format!(
        "BEGIN;
        {added}
        CREATE TABLE IF NOT EXISTS device_traces (
            device_id INTEGER PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
            kind TEXT,
            node_id INTEGER NOT NULL,
            endpoint INTEGER NOT NULL,
            network TEXT,
            software_version TEXT,
            thread_parent TEXT,
            seen_at INTEGER NOT NULL
        );
        PRAGMA user_version = 13;
        COMMIT;"
    ))
}

/// The statements adding those of `columns` that `light_outages` lacks.
fn missing_columns(db: &Connection, columns: &[(&str, &str)]) -> rusqlite::Result<String> {
    let have: Vec<String> = db
        .prepare("SELECT name FROM pragma_table_info('light_outages')")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(columns
        .iter()
        .filter(|(column, _)| !have.iter().any(|c| c == column))
        .fold(String::new(), |mut added, (column, kind)| {
            let _ = writeln!(
                added,
                "ALTER TABLE light_outages ADD COLUMN {column} {kind};"
            );
            added
        }))
}

/// Version 14 keeps home-link registering a light that left again: its tries, why it is not
/// back yet, when it was last heard and whether it is back, and how often it left before.
pub(crate) fn migrate_rejoin(db: &Connection) -> rusqlite::Result<()> {
    let added = missing_columns(
        db,
        &[
            ("rejoin_attempts", "INTEGER NOT NULL DEFAULT 0"),
            ("rejoin_at", "INTEGER"),
            ("rejoin_reason", "TEXT"),
            ("rejoin_reason_since", "INTEGER"),
            ("advert_at", "INTEGER"),
            ("rejoined", "INTEGER NOT NULL DEFAULT 0"),
            ("rejoin_told", "INTEGER NOT NULL DEFAULT 0"),
            ("leaves", "INTEGER NOT NULL DEFAULT 0"),
            ("previous_left_at", "INTEGER"),
        ],
    )?;
    db.execute_batch(&format!(
        "BEGIN;
        {added}
        PRAGMA user_version = 14;
        COMMIT;"
    ))
}

/// Version 15 keeps the tries in a row that lost the adapter, and until when they slow the next.
pub(crate) fn migrate_adapter_losses(db: &Connection) -> rusqlite::Result<()> {
    let added = missing_columns(
        db,
        &[
            ("rejoin_adapter_losses", "INTEGER NOT NULL DEFAULT 0"),
            ("rejoin_slowed_until", "INTEGER"),
        ],
    )?;
    db.execute_batch(&format!(
        "BEGIN;
        {added}
        PRAGMA user_version = 15;
        COMMIT;"
    ))
}

fn network_of(text: Option<&str>) -> Option<&'static str> {
    ["wifi", "thread", "ethernet"]
        .into_iter()
        .find(|network| text == Some(*network))
}

fn open_outages(db: &Connection) -> rusqlite::Result<Vec<Outage>> {
    db.prepare(
        "SELECT node_id, endpoint, name, network, started_at, found_by, reconnects,
            reconnect_result, reachable, notified, advised, device_id, left_at, left_by, labels,
            serial_number, software_version, thread_parent, left_told, rejoin_attempts, rejoin_at,
            rejoin_reason, rejoin_reason_since, advert_at, rejoined, rejoin_told, leaves,
            previous_left_at, rejoin_adapter_losses, rejoin_slowed_until
         FROM light_outages WHERE ended_at IS NULL",
    )?
    .query_map([], |row| {
        let key = row
            .get::<_, Option<i64>>(0)?
            .and_then(|node| u64::try_from(node).ok())
            .zip(row.get::<_, Option<u16>>(1)?);
        let found_by = match row.get::<_, String>(5)?.as_str() {
            "no_answer" => Cause::NoAnswer,
            "unreachable" => Cause::Unreachable,
            "left_fabric" => Cause::Left,
            _ => Cause::Unavailable,
        };
        let mut outage = Outage::new(
            key,
            &row.get::<_, String>(2)?,
            network_of(row.get::<_, Option<String>>(3)?.as_deref()),
            row.get(4)?,
            found_by,
        )
        .told(row.get(9)?, row.get(10)?)
        .told_left(row.get(18)?);
        outage.reconnects = row.get(6)?;
        outage.reconnect_result = row.get(7)?;
        outage.reachable = row.get(8)?;
        outage.device_id = row.get(11)?;
        outage.left_at = row.get(12)?;
        outage.left_by = match row.get::<_, Option<String>>(13)?.as_deref() {
            Some("event") => Some(LeftBy::Event),
            Some("check") => Some(LeftBy::Check),
            Some(_) => Some(LeftBy::Unknown),
            None => None,
        };
        outage.labels = serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or_default();
        outage.serial_number = row.get(15)?;
        outage.software_version = row.get(16)?;
        outage.thread_parent = row.get(17)?;
        outage.rejoin = Pursuit {
            attempts: row.get(19)?,
            attempted_at: row.get(20)?,
            reason: row
                .get::<_, Option<String>>(21)?
                .as_deref()
                .and_then(Reason::parse),
            reason_since: row.get(22)?,
            advert_at: row.get(23)?,
            rejoined: row.get(24)?,
            told: row.get(25)?,
            adapter_losses: row.get(28)?,
            slowed_until: row.get(29)?,
        };
        outage.leaves = Leaves {
            count: row.get(26)?,
            previous_at: row.get(27)?,
        };
        Ok(outage)
    })?
    .collect()
}

/// Takes back the stretches still open when home-link last stopped, so what was told is not
/// told again and a light back is told back.
pub(crate) fn resume(state: &AppState) {
    match open_outages(&state.db.lock().expect(UNPOISONED)) {
        Ok(outages) => state.watch.lock().expect(UNPOISONED).resume(outages),
        Err(error) => tracing::warn!(%error, "open light stretches not taken back"),
    }
}

fn key_text(key: Option<Key>) -> String {
    key.map_or_else(
        || "matterjs-server".into(),
        |(node, endpoint)| format!("{node}/{endpoint}"),
    )
}

/// The stretch's key in the history: its light's node and endpoint, else its ledger row for a
/// light that left before the watch saw it, else matterjs-server.
fn outage_key(outage: &Outage) -> String {
    match (outage.key(), outage.device_id) {
        (None, Some(device)) => format!("device/{device}"),
        (key, _) => key_text(key),
    }
}

fn save(db: &Connection, outage: &Outage) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO light_outages (key, node_id, endpoint, name, network, started_at, ended_at,
            found_by, reconnects, reconnect_result, reachable, notified, advised, device_id,
            left_at, left_by, labels, serial_number, software_version, thread_parent, left_told,
            rejoin_attempts, rejoin_at, rejoin_reason, rejoin_reason_since, advert_at, rejoined,
            rejoin_told, leaves, previous_left_at, rejoin_adapter_losses, rejoin_slowed_until)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32)
         ON CONFLICT (key, started_at) DO UPDATE SET name = excluded.name,
            network = excluded.network, ended_at = excluded.ended_at,
            reconnects = excluded.reconnects, reconnect_result = excluded.reconnect_result,
            reachable = excluded.reachable, notified = excluded.notified,
            advised = excluded.advised, device_id = excluded.device_id,
            left_at = excluded.left_at, left_by = excluded.left_by, labels = excluded.labels,
            serial_number = excluded.serial_number, software_version = excluded.software_version,
            thread_parent = excluded.thread_parent, left_told = excluded.left_told,
            rejoin_attempts = excluded.rejoin_attempts, rejoin_at = excluded.rejoin_at,
            rejoin_reason = excluded.rejoin_reason,
            rejoin_reason_since = excluded.rejoin_reason_since, advert_at = excluded.advert_at,
            rejoined = excluded.rejoined, rejoin_told = excluded.rejoin_told,
            leaves = excluded.leaves, previous_left_at = excluded.previous_left_at,
            rejoin_adapter_losses = excluded.rejoin_adapter_losses,
            rejoin_slowed_until = excluded.rejoin_slowed_until",
        params![
            outage_key(outage),
            outage
                .node_id
                .map(|node| i64::try_from(node).unwrap_or(i64::MAX)),
            outage.endpoint,
            outage.name,
            outage.network,
            outage.started_at,
            outage.ended_at,
            outage.found_by.as_str(),
            outage.reconnects,
            outage.reconnect_result,
            outage.reachable,
            outage.notified(),
            outage.advised(),
            outage.device_id,
            outage.left_at,
            outage.left_by.map(LeftBy::as_str),
            Value::from(outage.labels.clone()).to_string(),
            outage.serial_number,
            outage.software_version,
            outage.thread_parent,
            outage.left_told(),
            outage.rejoin.attempts,
            outage.rejoin.attempted_at,
            outage.rejoin.reason.map(Reason::as_str),
            outage.rejoin.reason_since,
            outage.rejoin.advert_at,
            outage.rejoin.rejoined,
            outage.rejoin.told,
            outage.leaves.count,
            outage.leaves.previous_at,
            outage.rejoin.adapter_losses,
            outage.rejoin.slowed_until,
        ],
    )?;
    Ok(())
}

/// Keeps `change` in the history; its own module logs it.
pub(crate) fn keep(state: &AppState, change: &Change) {
    let (Change::Down(outage) | Change::Updated(outage) | Change::Up(outage)) = change;
    if let Err(error) = save(&state.db.lock().expect(UNPOISONED), outage) {
        tracing::warn!(%error, "light history not saved");
    }
}

/// Logs `change` on one line, and keeps it in the history.
fn record(state: &AppState, change: &Change) {
    let (Change::Down(outage) | Change::Updated(outage) | Change::Up(outage)) = change;
    let (node_id, endpoint) = (outage.node_id, outage.endpoint);
    let name = outage.name.as_str();
    match (change, outage.key()) {
        // A light leaving the fabric: told by Down when it was up, by Updated when it was down.
        (Change::Down(_) | Change::Updated(_), _)
            if outage.left_at.is_some() && !outage.left_told() =>
        {
            tracing::warn!(
                name,
                device_id = outage.device_id,
                node_id,
                endpoint,
                serial_number = outage.serial_number.as_deref().unwrap_or("-"),
                left_at = outage.left_at.map(crate::schedule::utc),
                left_by = outage.left_by.map_or("-", crate::watch::LeftBy::as_str),
                software_version = outage.software_version.as_deref().unwrap_or("-"),
                thread_parent = outage.thread_parent.as_deref().unwrap_or("-"),
                "light left the fabric"
            );
        }
        (Change::Down(_), Some(_)) => tracing::info!(
            name,
            node_id,
            endpoint,
            network = outage.network.unwrap_or("-"),
            found_by = outage.found_by.as_str(),
            "light down"
        ),
        (Change::Down(_), None) => tracing::warn!("matterjs-server out of reach"),
        (Change::Up(_), Some(_)) => tracing::info!(
            name,
            node_id,
            endpoint,
            down_s = outage.ended_at.unwrap_or(outage.started_at) - outage.started_at,
            reconnects = outage.reconnects,
            "light back"
        ),
        (Change::Up(_), None) => tracing::info!(
            down_s = outage.ended_at.unwrap_or(outage.started_at) - outage.started_at,
            "matterjs-server back"
        ),
        (Change::Updated(_), _) => {}
    }
    if let Err(error) = save(&state.db.lock().expect(UNPOISONED), outage) {
        tracing::warn!(%error, "light history not saved");
    }
}

/// Keeps which lights this check found connected, and drops what is older than two weeks.
fn keep_check(state: &AppState, at: i64) -> rusqlite::Result<()> {
    let (server, lights) = {
        let watch = state.watch.lock().expect(UNPOISONED);
        let lights: Vec<Value> = watch
            .connected()
            .into_iter()
            .map(|(key, device, name, state)| {
                json!([
                    key.map(|k| k.0),
                    key.map(|k| k.1),
                    name,
                    state == "connected",
                    device,
                    state
                ])
            })
            .collect();
        (watch.status().server_reachable == Some(true), lights)
    };
    let db = state.db.lock().expect(UNPOISONED);
    db.execute(
        "INSERT OR REPLACE INTO light_checks (at, server, lights) VALUES (?1, ?2, ?3)",
        params![at, server, Value::from(lights).to_string()],
    )?;
    db.execute("DELETE FROM light_checks WHERE at < ?1", [at - KEPT])?;
    // A stretch still going is kept however long it has lasted.
    db.execute("DELETE FROM light_outages WHERE ended_at < ?1", [at - KEPT])?;
    Ok(())
}

/// What a ledger device last was while matterjs-server served it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Trace {
    kind: Option<&'static str>,
    node_id: u64,
    endpoint: u16,
    network: Option<&'static str>,
    software_version: Option<String>,
    thread_parent: Option<String>,
}

/// What each ledger device matterjs-server serves is now.
fn traces_of(devices: &[Device], nodes: &[Value]) -> Vec<(i64, Trace)> {
    devices
        .iter()
        .filter_map(|device| {
            let (node_id, endpoint) = place(device, nodes)?;
            let network = matter::network(nodes, node_id);
            Some((
                device.id,
                Trace {
                    kind: matter::kind(nodes, node_id, endpoint).map(|(kind, _)| kind.as_str()),
                    node_id,
                    endpoint,
                    network,
                    software_version: matter::software_version(nodes, node_id, endpoint),
                    thread_parent: (network == Some("thread"))
                        .then(|| matter::thread_parent(nodes, node_id))
                        .flatten(),
                },
            ))
        })
        .collect()
}

/// Keeps what each ledger device served is now, and reads back what every device last was.
fn keep_traces(
    db: &Connection,
    traces: &[(i64, Trace)],
    at: i64,
) -> rusqlite::Result<BTreeMap<i64, Trace>> {
    for (device, trace) in traces {
        db.execute(
            "INSERT OR REPLACE INTO device_traces (device_id, kind, node_id, endpoint, network,
                software_version, thread_parent, seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                device,
                trace.kind,
                i64::try_from(trace.node_id).unwrap_or(i64::MAX),
                trace.endpoint,
                trace.network,
                trace.software_version,
                trace.thread_parent,
                at,
            ],
        )?;
    }
    db.prepare(
        "SELECT device_id, kind, node_id, endpoint, network, software_version, thread_parent
         FROM device_traces",
    )?
    .query_map([], |row| {
        let kind = row.get::<_, Option<String>>(1)?;
        Ok((
            row.get(0)?,
            Trace {
                kind: ["light", "plug", "aircon", "other"]
                    .into_iter()
                    .find(|k| kind.as_deref() == Some(*k)),
                node_id: u64::try_from(row.get::<_, i64>(2)?).unwrap_or_default(),
                endpoint: row.get(3)?,
                network: network_of(row.get::<_, Option<String>>(4)?.as_deref()),
                software_version: row.get(5)?,
                thread_parent: row.get(6)?,
            },
        ))
    })?
    .collect()
}

/// When each ledger device left the fabric, as far as the history goes.
fn leaves(db: &Connection) -> rusqlite::Result<BTreeMap<i64, Vec<i64>>> {
    let mut leaves: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let mut statement = db.prepare(
        "SELECT device_id, left_at FROM light_outages
         WHERE device_id IS NOT NULL AND left_at IS NOT NULL ORDER BY left_at",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (device, at) = row?;
        leaves.entry(device).or_default().push(at);
    }
    Ok(leaves)
}

/// The ledger lights matterjs-server does not serve: each device once served, by its serial
/// number, that it serves no more and that was last a light or is not known to be anything
/// else, with what it last was and when matterjs-server sent it removed the node.
fn left_lights(
    devices: &[Device],
    nodes: &[Value],
    traces: &BTreeMap<i64, Trace>,
    removed: &[(u64, i64)],
    leaves: &BTreeMap<i64, Vec<i64>>,
) -> Vec<Left> {
    devices
        .iter()
        .filter(|device| crate::is_missing(device, nodes))
        .filter_map(|device| {
            let trace = traces.get(&device.id);
            if trace.is_some_and(|trace| trace.kind.is_some_and(|kind| kind != "light")) {
                return None;
            }
            Some(Left {
                device: device.id,
                name: device.name.clone(),
                labels: device
                    .labels
                    .iter()
                    .map(|label| label.name.clone())
                    .collect(),
                serial: device.serial_number.clone(),
                ignored: device.ignored,
                network: trace.and_then(|trace| trace.network),
                software_version: trace.and_then(|trace| trace.software_version.clone()),
                thread_parent: trace.and_then(|trace| trace.thread_parent.clone()),
                leaves: leaves.get(&device.id).cloned().unwrap_or_default(),
                removed_at: trace.and_then(|trace| {
                    removed
                        .iter()
                        .filter(|(node, _)| *node == trace.node_id)
                        .map(|(_, at)| *at)
                        .max()
                }),
            })
        })
        .collect()
}

/// What one check saw: every light with its name, ledger row, availability and, when due,
/// whether it answered a read, and the ledger lights not served; `Ok(None)` when
/// matterjs-server was out of reach.
///
/// # Errors
/// When the ledger cannot be read: which lights it ignores is not known, so nothing is judged.
async fn observe(
    state: &AppState,
    url: &str,
    at: i64,
) -> rusqlite::Result<Option<(Vec<Seen>, Vec<Left>)>> {
    let deadline = Instant::now() + CHECK_TIMEOUT;
    let Ok(mut link) = Link::open(url, deadline).await else {
        return Ok(None);
    };
    let Ok(nodes) = link.nodes(deadline).await else {
        link.close().await;
        return Ok(None);
    };
    let lights = matter::lights(&nodes);
    let read = {
        let db = state.db.lock().expect(UNPOISONED);
        ledger(&db).and_then(|devices| {
            let traces = keep_traces(&db, &traces_of(&devices, &nodes), at)?;
            Ok((devices, traces, leaves(&db)?))
        })
    };
    let (devices, traces, leaves) = match read {
        Ok(read) => read,
        Err(error) => {
            link.close().await;
            return Err(error);
        }
    };
    let names = match_ledger(&devices, &nodes, &lights).0;
    let left = left_lights(
        &devices,
        &nodes,
        &traces,
        &state.removed.lock().expect(UNPOISONED),
        &leaves,
    );
    let mut seen: Vec<Seen> = lights
        .iter()
        .zip(names)
        .map(|(light, (name, ignored))| Seen {
            key: (light.node_id, light.endpoint),
            name: name.unwrap_or_else(|| format!("node {}/{}", light.node_id, light.endpoint)),
            network: matter::network(&nodes, light.node_id),
            device: devices_of(&devices, &nodes, light)
                .next()
                .map(|device| device.id),
            ignored,
            available: light.reachable,
            answered: None,
        })
        .collect();
    let reads: Vec<(usize, Value)> = {
        let watch = state.watch.lock().expect(UNPOISONED);
        seen.iter()
            .enumerate()
            .filter(|(_, light)| !light.ignored && light.available && watch.read_due(at, light.key))
            .map(|(i, _)| (i, matter::read_on_off(&lights[i])))
            .collect()
    };
    if !reads.is_empty() {
        let by = deadline.min(Instant::now() + READ_TIMEOUT);
        match link.exchange(reads, by).await {
            Ok(replies) => {
                for (i, reply) in replies {
                    seen[i].answered = Some(matches!(
                        Outcome::of(reply.as_ref()),
                        Outcome::Accepted | Outcome::Refused(_)
                    ));
                }
            }
            // Unread this time: the lights are judged by their availability alone.
            Err(fault) => tracing::warn!(%fault, "light reads not completed"),
        }
    }
    link.close().await;
    Ok(Some((seen, left)))
}

/// Keeps that matterjs-server removed `node` at `at`.
fn removed(state: &AppState, node: u64, at: i64) {
    tracing::info!(
        node_id = node,
        at = crate::schedule::utc(at),
        "matterjs-server removed a node"
    );
    let mut removed = state.removed.lock().expect(UNPOISONED);
    removed.push((node, at));
    let over = removed.len().saturating_sub(MAX_REMOVED);
    removed.drain(..over);
}

/// Listens to matterjs-server for the nodes it removes, as when a light leaves the fabric, on
/// one connection opened again whenever it fails or stays quiet too long.
pub(crate) async fn listen(state: &AppState) {
    let Some(url) = state.matter_url.as_deref() else {
        return std::future::pending().await;
    };
    loop {
        if let Err(fault) = listen_once(state, url).await {
            tracing::debug!(%fault, "listening to matterjs-server again");
        }
        tokio::time::sleep(LISTEN_AGAIN).await;
    }
}

async fn listen_once(state: &AppState, url: &str) -> Result<(), crate::link::Fault> {
    let mut link = Link::open(url, Instant::now() + OPEN_TIMEOUT).await?;
    let request = json!({ "message_id": next_id(), "command": "start_listening" });
    link.send(&request, Instant::now() + OPEN_TIMEOUT).await?;
    while let Some(message) = link.next(Instant::now() + LISTEN_IDLE).await? {
        if message["event"] != "node_removed" {
            continue;
        }
        let node = message["data"]
            .as_u64()
            .or_else(|| message["data"].as_str()?.parse().ok());
        if let Some(node) = node {
            removed(state, node, (state.clock)());
        }
    }
    link.close().await;
    Ok(())
}

/// What a request to matterjs-server about a device came to, in a word and its details.
fn told(result: Result<Option<Value>, crate::link::Fault>) -> String {
    match result {
        Err(fault) => fault.to_string(),
        Ok(reply) => match Outcome::of(reply.as_ref()) {
            Outcome::Accepted => "accepted".into(),
            outcome => match outcome.details() {
                Some(details) => format!("{}: {details}", outcome.reason().unwrap_or_default()),
                None => outcome.reason().unwrap_or_default().into(),
            },
        },
    }
}

/// Sends `request` to matterjs-server on a connection of its own, by `timeout`.
async fn ask(
    url: &str,
    request: Value,
    timeout: Duration,
) -> Result<Option<Value>, crate::link::Fault> {
    let deadline = Instant::now() + timeout;
    let mut link = Link::open(url, deadline).await?;
    let reply = link.request(request, deadline).await;
    link.close().await;
    reply
}

/// Asks matterjs-server to read the whole of `node` again over a new session, which does not
/// change the light, and records what it came to for each light of the node in `keys`.
async fn reconnect(state: &AppState, url: &str, node: u64, keys: Vec<(Key, String)>) {
    for (key, name) in &keys {
        tracing::info!(
            name,
            node_id = key.0,
            endpoint = key.1,
            "light reconnect asked"
        );
    }
    let request = json!({ "command": "interview_node", "args": { "node_id": node } });
    let result = told(ask(url, request, RECONNECT_TIMEOUT).await);
    for (key, name) in keys {
        tracing::info!(
            name,
            node_id = key.0,
            endpoint = key.1,
            result,
            "light reconnect done"
        );
        let change = state
            .watch
            .lock()
            .expect(UNPOISONED)
            .reconnected(key, result.clone());
        if let Some(change) = change {
            record(state, &change);
        }
    }
}

/// Pings the addresses matterjs-server knows for `key`'s node, from where it runs, and records
/// whether any answered: a Wi-Fi light off the network answers none.
async fn ping(state: &AppState, url: &str, key: Key, name: String) {
    let request = json!({ "command": "ping_node", "args": { "node_id": key.0, "attempts": 1 } });
    let reply = ask(url, request, PING_TIMEOUT).await;
    let reachable = match &reply {
        Ok(Some(reply)) if reply.get("error_code").is_none() => reply["result"]
            .as_object()
            .map(|addresses| addresses.values().any(|ok| ok == true)),
        _ => None,
    };
    tracing::info!(
        name,
        node_id = key.0,
        endpoint = key.1,
        reachable = reachable.map_or("unknown", |ok| if ok { "yes" } else { "no" }),
        "light ping"
    );
    let change = reachable.and_then(|ok| state.watch.lock().expect(UNPOISONED).reached(key, ok));
    if let Some(change) = change {
        record(state, &change);
    }
}

fn notice_kind(notice: &Notice) -> &'static str {
    match notice {
        Notice::Down(_) => "down",
        Notice::StillDown(_) => "still_down",
        Notice::Up(_) => "up",
        Notice::Left(_) => "left_fabric",
        Notice::Rejoined(_) => "rejoined",
        Notice::NotRejoined(_) => "not_rejoined",
        Notice::ServerDown { .. } => "server_down",
        Notice::ServerUp { .. } => "server_up",
        Notice::Meter(notice) => notice.kind(),
    }
}

async fn send(state: &AppState, notice: &Notice) -> Result<(), String> {
    let discord = state.discord.as_ref().ok_or("no destination")?;
    let kind = notice_kind(notice);
    match discord.send(&notice.text()).await {
        Ok(()) => {
            tracing::info!(kind, "notice sent");
            Ok(())
        }
        Err(error) => {
            tracing::warn!(kind, %error, "notice not sent");
            Err(error)
        }
    }
}

/// One check at `at`, on the minute, and whatever it calls for.
pub(crate) async fn check(state: &AppState, at: i64) {
    let Some(url) = state.matter_url.as_deref() else {
        return;
    };
    let seen = match observe(state, url, at).await {
        Ok(seen) => seen,
        Err(error) => {
            tracing::warn!(%error, "ledger not read; lights not checked");
            return;
        }
    };
    let (seen, left) = seen.unzip();
    let step = state.watch.lock().expect(UNPOISONED).step(
        at,
        seen.as_deref(),
        left.as_deref().unwrap_or_default(),
        state.discord.is_some(),
    );
    for change in &step.changes {
        record(state, change);
    }
    if let Err(error) = keep_check(state, at) {
        tracing::warn!(%error, "light check not saved");
    }
    let name = |key: &Key| {
        seen.iter()
            .flatten()
            .find(|light| light.key == *key)
            .map_or_else(String::new, |light| light.name.clone())
    };
    // A bridge's lights are one node: it is read again once.
    let mut nodes: Vec<(u64, Vec<(Key, String)>)> = Vec::new();
    for key in &step.reconnect {
        match nodes.iter_mut().find(|(node, _)| *node == key.0) {
            Some((_, keys)) => keys.push((*key, name(key))),
            None => nodes.push((key.0, vec![(*key, name(key))])),
        }
    }
    let reconnects = nodes
        .into_iter()
        .map(|(node, keys)| reconnect(state, url, node, keys));
    let pings = step
        .ping
        .iter()
        .map(|key| ping(state, url, *key, name(key)));
    tokio::join!(
        futures_util::future::join_all(reconnects),
        futures_util::future::join_all(pings),
        deliver(state, step.notices),
    );
}

/// Sends the notices left unsent, oldest first, then `notices`; those that fail wait for the next check.
pub(crate) async fn deliver(state: &AppState, notices: Vec<Notice>) {
    let mut queue = std::mem::take(&mut *state.unsent.lock().expect(UNPOISONED));
    queue.extend(notices);
    let mut unsent = Vec::new();
    for notice in queue {
        if send(state, &notice).await.is_err() {
            unsent.push(notice);
        }
    }
    let mut kept = state.unsent.lock().expect(UNPOISONED);
    unsent.append(&mut kept);
    let over = unsent.len().saturating_sub(MAX_UNSENT);
    if over > 0 {
        tracing::warn!(dropped = over, "unsent notices dropped");
    }
    *kept = unsent.split_off(over);
}

/// The watch's state: matterjs-server, each light, and whether messages have somewhere to go.
pub(crate) async fn status(State(state): State<Db>) -> Json<Value> {
    let status = state.watch.lock().expect(UNPOISONED).status();
    let mut value = serde_json::to_value(status).unwrap_or_default();
    value["notify"] = json!(state.discord.is_some());
    Json(value)
}

#[derive(Deserialize)]
pub(crate) struct OutagesQuery {
    since: Option<i64>,
    node_id: Option<u64>,
    endpoint: Option<u16>,
}

/// The stretches lights and matterjs-server were out of reach, oldest first, that had not
/// ended by `since` (Unix seconds), of one light if `node_id` and `endpoint` are given.
pub(crate) async fn outages(
    State(state): State<Db>,
    Query(query): Query<OutagesQuery>,
) -> ApiResult<Json<Value>> {
    let key = match (query.node_id, query.endpoint) {
        (Some(node), Some(endpoint)) => Some(key_text(Some((node, endpoint)))),
        (None, None) => None,
        _ => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_query",
                "node_idとendpointは両方を指定してください".into(),
            ));
        }
    };
    let db = state.db.lock().expect(UNPOISONED);
    let mut statement = db.prepare(
        "SELECT node_id, endpoint, name, network, started_at, ended_at, found_by, reconnects,
            reconnect_result, reachable, device_id, left_at, left_by, labels, serial_number,
            software_version, thread_parent, rejoin_attempts, rejoin_reason, rejoined, leaves,
            previous_left_at
         FROM light_outages
         WHERE COALESCE(ended_at, ?1) >= ?1 AND (?2 IS NULL OR key = ?2)
         ORDER BY started_at, rowid",
    )?;
    let outages = statement
        .query_map(params![query.since.unwrap_or(0), key], |row| {
            Ok(json!({
                "node_id": row.get::<_, Option<i64>>(0)?,
                "endpoint": row.get::<_, Option<u16>>(1)?,
                "name": row.get::<_, String>(2)?,
                "network": row.get::<_, Option<String>>(3)?,
                "started_at": row.get::<_, i64>(4)?,
                "ended_at": row.get::<_, Option<i64>>(5)?,
                "found_by": row.get::<_, String>(6)?,
                "reconnects": row.get::<_, u32>(7)?,
                "reconnect_result": row.get::<_, Option<String>>(8)?,
                "reachable": row.get::<_, Option<bool>>(9)?,
                "device_id": row.get::<_, Option<i64>>(10)?,
                "left_at": row.get::<_, Option<i64>>(11)?,
                "left_by": row.get::<_, Option<String>>(12)?,
                "labels": serde_json::from_str::<Value>(&row.get::<_, String>(13)?).unwrap_or_default(),
                "serial_number": row.get::<_, Option<String>>(14)?,
                "software_version": row.get::<_, Option<String>>(15)?,
                "thread_parent": row.get::<_, Option<String>>(16)?,
                "rejoin_attempts": row.get::<_, u32>(17)?,
                "rejoin_reason": row.get::<_, Option<String>>(18)?,
                "rejoined": row.get::<_, bool>(19)?,
                "leaves": row.get::<_, u32>(20)?,
                "previous_left_at": row.get::<_, Option<i64>>(21)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({ "outages": outages })))
}

#[derive(Deserialize)]
pub(crate) struct ConnectedQuery {
    at: i64,
}

/// Which lights were connected at `at` (Unix seconds), as the check of that minute found them.
pub(crate) async fn connected(
    State(state): State<Db>,
    Query(query): Query<ConnectedQuery>,
) -> ApiResult<Json<Value>> {
    let check: Option<(i64, bool, String)> = state
        .db
        .lock()
        .expect(UNPOISONED)
        .query_row(
            "SELECT at, server, lights FROM light_checks WHERE at <= ?1 AND at > ?2
             ORDER BY at DESC LIMIT 1",
            params![query.at, query.at - CHECK_COVERS],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((checked_at, server, lights)) = check else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "not_checked",
            "その時刻には照明を確かめていません（記録は14日分です）".into(),
        ));
    };
    // Before version 0.1.57 a light was [node, endpoint, name, connected]; a light that left
    // the fabric, with no node, is there since, with its ledger row and state.
    let lights: Vec<Value> = serde_json::from_str::<Vec<Vec<Value>>>(&lights)
        .unwrap_or_default()
        .into_iter()
        .map(|light| {
            let field = |i: usize| light.get(i).cloned().unwrap_or(Value::Null);
            let connected = server && field(3) == true;
            let state = match field(5).as_str() {
                Some("left_fabric") => "left_fabric",
                _ if connected => "connected",
                _ => "down",
            };
            json!({
                "node_id": field(0),
                "endpoint": field(1),
                "name": field(2),
                "connected": connected,
                "device_id": field(4),
                "state": state,
            })
        })
        .collect();
    Ok(Json(json!({
        "at": query.at,
        "checked_at": checked_at,
        "server_reachable": server,
        "lights": lights,
    })))
}
