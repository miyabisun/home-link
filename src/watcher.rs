//! The light watch at work: a check every minute against matterjs-server, the reconnections,
//! pings and messages it calls for, and two weeks of history. [`crate::watch`] decides; this
//! module only carries the decisions out.

use std::time::Duration;

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
    ApiError, ApiResult, AppState, Db, UNPOISONED, ledger,
    link::{Link, Outcome},
    match_ledger, matter,
    watch::{Change, Key, Notice, Outage, Seen},
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
/// A check this close before a moment answers for it, in seconds: a minute and its slack.
const CHECK_COVERS: i64 = 90;

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

/// Ends the stretches left open when home-link last stopped, at its last check.
pub(crate) fn close_open(state: &AppState) {
    let closed = state.db.lock().expect(UNPOISONED).execute(
        "UPDATE light_outages
         SET ended_at = MAX(started_at, COALESCE((SELECT MAX(at) FROM light_checks), started_at))
         WHERE ended_at IS NULL",
        [],
    );
    if let Err(error) = closed {
        tracing::warn!(%error, "open light stretches not ended");
    }
}

fn key_text(key: Option<Key>) -> String {
    key.map_or_else(
        || "matterjs-server".into(),
        |(node, endpoint)| format!("{node}/{endpoint}"),
    )
}

fn save(db: &Connection, outage: &Outage) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO light_outages (key, node_id, endpoint, name, network, started_at, ended_at,
            found_by, reconnects, reconnect_result, reachable)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (key, started_at) DO UPDATE SET name = excluded.name,
            ended_at = excluded.ended_at, reconnects = excluded.reconnects,
            reconnect_result = excluded.reconnect_result, reachable = excluded.reachable",
        params![
            key_text(outage.key()),
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
        ],
    )?;
    Ok(())
}

/// Logs `change` on one line, and keeps it in the history.
fn record(state: &AppState, change: &Change) {
    let (Change::Down(outage) | Change::Updated(outage) | Change::Up(outage)) = change;
    let (node_id, endpoint) = (outage.node_id, outage.endpoint);
    let name = outage.name.as_str();
    match (change, outage.key()) {
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
            .map(|((node, endpoint), name, connected)| json!([node, endpoint, name, connected]))
            .collect();
        (watch.status().server_reachable == Some(true), lights)
    };
    let db = state.db.lock().expect(UNPOISONED);
    db.execute(
        "INSERT OR REPLACE INTO light_checks (at, server, lights) VALUES (?1, ?2, ?3)",
        params![at, server, Value::from(lights).to_string()],
    )?;
    db.execute("DELETE FROM light_checks WHERE at < ?1", [at - KEPT])?;
    db.execute(
        "DELETE FROM light_outages WHERE COALESCE(ended_at, started_at) < ?1",
        [at - KEPT],
    )?;
    Ok(())
}

/// What one check saw: every light with its name, availability and, when due, whether it
/// answered a read; `None` when matterjs-server was out of reach.
async fn observe(state: &AppState, url: &str, at: i64) -> Option<Vec<Seen>> {
    let deadline = Instant::now() + CHECK_TIMEOUT;
    let mut link = Link::open(url, deadline).await.ok()?;
    let Ok(nodes) = link.nodes(deadline).await else {
        link.close().await;
        return None;
    };
    let lights = matter::lights(&nodes);
    let names = match ledger(&state.db.lock().expect(UNPOISONED)) {
        Ok(devices) => match_ledger(&devices, &nodes, &lights).0,
        Err(error) => {
            tracing::warn!(%error, "ledger not read for the light watch");
            vec![(None, false); lights.len()]
        }
    };
    let mut seen: Vec<Seen> = lights
        .iter()
        .zip(names)
        .map(|(light, (name, ignored))| Seen {
            key: (light.node_id, light.endpoint),
            name: name.unwrap_or_else(|| format!("node {}/{}", light.node_id, light.endpoint)),
            network: matter::network(&nodes, light.node_id),
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
    Some(seen)
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
        Notice::ServerDown { .. } => "server_down",
        Notice::ServerUp { .. } => "server_up",
        Notice::Test => "test",
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
    let seen = observe(state, url, at).await;
    let step =
        state
            .watch
            .lock()
            .expect(UNPOISONED)
            .step(at, seen.as_deref(), state.discord.is_some());
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
    let notices = step.notices.iter().map(|notice| async move {
        let _ = send(state, notice).await;
    });
    tokio::join!(
        futures_util::future::join_all(reconnects),
        futures_util::future::join_all(pings),
        futures_util::future::join_all(notices),
    );
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
            reconnect_result, reachable
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
    let lights: Vec<Value> = serde_json::from_str::<Vec<(u64, u16, String, bool)>>(&lights)
        .unwrap_or_default()
        .into_iter()
        .map(|(node_id, endpoint, name, connected)| {
            json!({ "node_id": node_id, "endpoint": endpoint, "name": name, "connected": server && connected })
        })
        .collect();
    Ok(Json(json!({
        "at": query.at,
        "checked_at": checked_at,
        "server_reachable": server,
        "lights": lights,
    })))
}

/// Sends a test message to the destination, so its setting can be checked without a light going down.
pub(crate) async fn test_notice(State(state): State<Db>) -> ApiResult<Json<Value>> {
    if state.discord.is_none() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "notify_not_configured",
            format!(
                "知らせの送り先（{}）が設定されていません",
                crate::discord::ENV
            ),
        ));
    }
    send(&state, &Notice::Test).await.map_err(|_| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            "notify_failed",
            "知らせを送れませんでした。送り先の設定を確かめてください".into(),
        )
    })?;
    Ok(Json(json!({ "sent": true })))
}
