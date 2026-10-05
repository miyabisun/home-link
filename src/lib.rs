pub mod matter;
pub mod onboarding;

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::trace::TraceLayer;

const MAX_NAME_CHARS: usize = 100;
const MAX_QR_CHARS: usize = 512;

/// Opens (or creates) the `SQLite` database and applies the schema.
///
/// # Errors
/// Fails when the file cannot be opened or the schema cannot be applied.
pub fn open_db(path: &str) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS rooms (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );
        CREATE TABLE IF NOT EXISTS devices (
            id INTEGER PRIMARY KEY,
            room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
            qr_payload TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );",
    )?;
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 1 {
        // Version 1 makes the setup code optional and adds the device's own identifiers.
        db.execute_batch(
            "BEGIN;
            CREATE TABLE devices_v1 (
                id INTEGER PRIMARY KEY,
                room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
                qr_payload TEXT UNIQUE,
                vendor TEXT,
                serial_number TEXT,
                mac TEXT UNIQUE,
                name TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (vendor, serial_number),
                CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
            );
            INSERT INTO devices_v1 (id, room_id, qr_payload, name, created_at)
                SELECT id, room_id, qr_payload, name, created_at FROM devices;
            DROP TABLE devices;
            ALTER TABLE devices_v1 RENAME TO devices;
            PRAGMA user_version = 1;
            COMMIT;",
        )?;
    }
    Ok(db)
}

struct AppState {
    db: Mutex<Connection>,
    matter_url: Option<String>,
}

type Db = Arc<AppState>;

/// Builds the router; `matter_url` is the matterjs-server WebSocket API used by the status API.
pub fn app(db: Connection, matter_url: Option<String>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/rooms", get(list_rooms).post(create_room))
        .route(
            "/rooms/{id}",
            axum::routing::patch(rename_room).delete(delete_room),
        )
        .route("/devices", get(list_devices).post(create_device))
        .route("/devices/{id}", get(get_device).delete(delete_device))
        .route("/status", get(status))
        .route("/lights", get(light_states))
        .route("/lights/on", axum::routing::post(lights_on))
        .route("/lights/off", axum::routing::post(lights_off))
        .fallback(not_found)
        // ponytail: one connection behind a global lock; a pool if writes ever contend.
        .with_state(Arc::new(AppState {
            db: Mutex::new(db),
            matter_url,
        }));

    Router::new()
        .route("/healthz", get(healthz))
        .nest("/api", api)
        .layer(TraceLayer::new_for_http())
}

struct ApiError(StatusCode, &'static str, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1, "message": self.2 }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        tracing::error!(%error, "database error");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "サーバーでエラーが発生しました".into(),
        )
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn room_not_found() -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        "room_not_found",
        "部屋が見つかりません".into(),
    )
}

fn is_unique_violation(error: &rusqlite::Error) -> bool {
    error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation)
}

/// Trims `name` and checks its length; `allow_empty` permits a blank result.
fn clean_name(name: &str, allow_empty: bool) -> Option<String> {
    let name = name.trim();
    let ok = (allow_empty || !name.is_empty()) && name.chars().count() <= MAX_NAME_CHARS;
    ok.then(|| name.to_owned())
}

async fn healthz() -> &'static str {
    "ok\n"
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn not_found() -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        "not_found",
        "APIが見つかりません".into(),
    )
}

#[derive(Serialize)]
struct Room {
    id: i64,
    name: String,
    device_count: i64,
}

#[derive(Deserialize)]
struct RoomInput {
    name: String,
}

fn find_room(db: &Connection, id: i64) -> rusqlite::Result<Option<Room>> {
    db.query_row(
        "SELECT id, name, (SELECT count(*) FROM devices WHERE room_id = rooms.id)
         FROM rooms WHERE id = ?1",
        [id],
        |row| {
            Ok(Room {
                id: row.get(0)?,
                name: row.get(1)?,
                device_count: row.get(2)?,
            })
        },
    )
    .optional()
}

fn valid_room_name(input: &RoomInput) -> ApiResult<String> {
    clean_name(&input.name, false).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_room_name",
            format!("部屋名は1〜{MAX_NAME_CHARS}文字で入力してください"),
        )
    })
}

fn duplicate_room_name() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "duplicate_room_name",
        "同じ名前の部屋があります".into(),
    )
}

async fn list_rooms(State(db): State<Db>) -> ApiResult<Json<Vec<Room>>> {
    let db = db.db.lock().unwrap();
    let mut statement = db.prepare(
        "SELECT id, name, (SELECT count(*) FROM devices WHERE room_id = rooms.id)
         FROM rooms ORDER BY id",
    )?;
    let rooms = statement
        .query_map([], |row| {
            Ok(Room {
                id: row.get(0)?,
                name: row.get(1)?,
                device_count: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Json(rooms))
}

async fn create_room(
    State(db): State<Db>,
    Json(input): Json<RoomInput>,
) -> ApiResult<(StatusCode, Json<Room>)> {
    let name = valid_room_name(&input)?;
    let db = db.db.lock().unwrap();
    match db.execute("INSERT INTO rooms (name) VALUES (?1)", [&name]) {
        Err(error) if is_unique_violation(&error) => Err(duplicate_room_name()),
        result => {
            result?;
            let room = find_room(&db, db.last_insert_rowid())?.ok_or_else(room_not_found)?;
            Ok((StatusCode::CREATED, Json(room)))
        }
    }
}

async fn rename_room(
    State(db): State<Db>,
    Path(id): Path<i64>,
    Json(input): Json<RoomInput>,
) -> ApiResult<Json<Room>> {
    let name = valid_room_name(&input)?;
    let db = db.db.lock().unwrap();
    match db.execute(
        "UPDATE rooms SET name = ?1 WHERE id = ?2",
        params![name, id],
    ) {
        Err(error) if is_unique_violation(&error) => Err(duplicate_room_name()),
        result => {
            result?;
            Ok(Json(find_room(&db, id)?.ok_or_else(room_not_found)?))
        }
    }
}

async fn delete_room(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let db = db.db.lock().unwrap();
    let room = find_room(&db, id)?.ok_or_else(room_not_found)?;
    if room.device_count > 0 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "room_has_devices",
            format!(
                "この部屋には機器が{}台登録されています。先に機器を削除してください",
                room.device_count
            ),
        ));
    }
    db.execute("DELETE FROM rooms WHERE id = ?1", [id])?;
    Ok(StatusCode::NO_CONTENT)
}

/// A device as listed: its setup code holds the setup passcode, so it is
/// returned only by the single-device endpoint.
#[derive(Serialize)]
struct Device {
    id: i64,
    room_id: i64,
    room_name: String,
    name: String,
    vendor: Option<String>,
    serial_number: Option<String>,
    mac: Option<String>,
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    qr_payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_code: Option<String>,
}

/// A device to register with at most one of its QR payload or manual pairing code,
/// and its own identifiers; a code or the vendor and serial number is required.
#[derive(Deserialize)]
struct DeviceInput {
    room_id: i64,
    qr_payload: Option<String>,
    manual_code: Option<String>,
    vendor: Option<String>,
    serial_number: Option<String>,
    mac: Option<String>,
    #[serde(default)]
    name: String,
}

const DEVICE_SELECT: &str = "SELECT devices.id, room_id, rooms.name, devices.name, created_at,
    qr_payload, vendor, serial_number, mac FROM devices JOIN rooms ON rooms.id = room_id";

/// The `qr_payload` column stores either form; a QR payload starts with `MT:`.
fn device_row(row: &rusqlite::Row, with_payload: bool) -> rusqlite::Result<Device> {
    let code: Option<String> = if with_payload { row.get(5)? } else { None };
    let (qr_payload, manual_code) = match code {
        Some(code) if !code.starts_with("MT:") => (None, Some(code)),
        code => (code, None),
    };
    Ok(Device {
        id: row.get(0)?,
        room_id: row.get(1)?,
        room_name: row.get(2)?,
        name: row.get(3)?,
        vendor: row.get(6)?,
        serial_number: row.get(7)?,
        mac: row.get(8)?,
        created_at: row.get(4)?,
        qr_payload,
        manual_code,
    })
}

fn find_device(db: &Connection, id: i64, with_payload: bool) -> ApiResult<Device> {
    db.query_row(
        &format!("{DEVICE_SELECT} WHERE devices.id = ?1"),
        [id],
        |row| device_row(row, with_payload),
    )
    .optional()?
    .ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            "device_not_found",
            "機器が見つかりません".into(),
        )
    })
}

async fn list_devices(State(db): State<Db>) -> ApiResult<Json<Vec<Device>>> {
    let db = db.db.lock().unwrap();
    let mut statement = db.prepare(&format!("{DEVICE_SELECT} ORDER BY devices.id"))?;
    let devices = statement
        .query_map([], |row| device_row(row, false))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Json(devices))
}

async fn get_device(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<Json<Device>> {
    Ok(Json(find_device(&db.db.lock().unwrap(), id, true)?))
}

fn missing_setup_code() -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        "missing_setup_code",
        "QRコードか手動ペアリングコードのどちらか一方、または機器の識別子（vendorとserial_number）を送ってください".into(),
    )
}

/// Validates the optional setup code of `input` and returns the form to store.
fn setup_code(input: &DeviceInput) -> ApiResult<Option<String>> {
    match (&input.qr_payload, &input.manual_code) {
        (None, None) => Ok(None),
        (Some(payload), None) => {
            let payload = payload.trim();
            (payload.len() <= MAX_QR_CHARS && onboarding::is_valid(payload))
                .then(|| Some(payload.to_owned()))
                .ok_or_else(|| {
                    ApiError(
                        StatusCode::BAD_REQUEST,
                        "invalid_qr_payload",
                        "MatterのQRコード（MT:で始まる）ではありません".into(),
                    )
                })
        }
        (None, Some(code)) => onboarding::normalize_manual(code.trim())
            .map(Some)
            .ok_or_else(|| {
                ApiError(
                    StatusCode::BAD_REQUEST,
                    "invalid_manual_code",
                    "Matterの手動ペアリングコード（11桁の数字）として正しくありません".into(),
                )
            }),
        (Some(_), Some(_)) => Err(missing_setup_code()),
    }
}

/// The device's own identifiers: `(vendor, serial_number)` and a Wi-Fi MAC.
type Identifiers = (Option<(String, String)>, Option<String>);

/// Validates the vendor and serial number (both or neither) and normalizes the MAC
/// to 12 upper-case hex digits.
fn identifiers(input: &DeviceInput) -> ApiResult<Identifiers> {
    let invalid = || {
        ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_identifier",
            format!("vendorとserial_numberは両方を1〜{MAX_NAME_CHARS}文字で送ってください"),
        )
    };
    let serial = match (&input.vendor, &input.serial_number) {
        (None, None) => None,
        (Some(vendor), Some(serial)) => Some((
            clean_name(vendor, false).ok_or_else(invalid)?,
            clean_name(serial, false).ok_or_else(invalid)?,
        )),
        _ => return Err(invalid()),
    };
    let mac = input
        .mac
        .as_deref()
        .map(|mac| {
            let hex: String = mac.chars().filter(|c| !matches!(c, ':' | '-')).collect();
            (hex.len() == 12 && hex.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| hex.to_ascii_uppercase())
                .ok_or_else(|| {
                    ApiError(
                        StatusCode::BAD_REQUEST,
                        "invalid_mac",
                        "MACアドレスは16進数12桁で送ってください".into(),
                    )
                })
        })
        .transpose()?;
    Ok((serial, mac))
}

fn duplicate_device() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "duplicate_qr_payload",
        "この機器は登録済みです".into(),
    )
}

/// Whether a stored device shares a passcode and short discriminator with `code`.
// ponytail: scans every device per insert; a key column if the ledger outgrows a home.
fn is_registered(db: &Connection, code: &str) -> rusqlite::Result<bool> {
    let new = onboarding::keys(code).unwrap_or_default();
    let mut statement = db.prepare("SELECT qr_payload FROM devices")?;
    let stored = statement
        .query_map([], |row| row.get::<_, Option<String>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(stored
        .iter()
        .flatten()
        .flat_map(|code| onboarding::keys(code).unwrap_or_default())
        .any(|key| new.contains(&key)))
}

async fn create_device(
    State(db): State<Db>,
    Json(input): Json<DeviceInput>,
) -> ApiResult<(StatusCode, Json<Device>)> {
    let code = setup_code(&input)?;
    let (serial, mac) = identifiers(&input)?;
    if code.is_none() && serial.is_none() {
        return Err(missing_setup_code());
    }
    let name = clean_name(&input.name, true).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
            format!("機器名は{MAX_NAME_CHARS}文字以内で入力してください"),
        )
    })?;
    let db = db.db.lock().unwrap();
    find_room(&db, input.room_id)?.ok_or_else(room_not_found)?;
    if let Some(code) = &code
        && is_registered(&db, code)?
    {
        return Err(duplicate_device());
    }
    let (vendor, serial_number) = serial.unzip();
    let taken: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM devices
            WHERE (vendor = ?1 AND serial_number = ?2) OR mac = ?3)",
        params![vendor, serial_number, mac],
        |row| row.get(0),
    )?;
    if taken {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "duplicate_identifier",
            "同じ識別子の機器が登録済みです".into(),
        ));
    }
    match db.execute(
        "INSERT INTO devices (room_id, qr_payload, vendor, serial_number, mac, name)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![input.room_id, code, vendor, serial_number, mac, name],
    ) {
        Err(error) if is_unique_violation(&error) => Err(duplicate_device()),
        result => {
            result?;
            let device = find_device(&db, db.last_insert_rowid(), false)?;
            Ok((StatusCode::CREATED, Json(device)))
        }
    }
}

async fn delete_device(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let db = db.db.lock().unwrap();
    find_device(&db, id, false)?;
    db.execute("DELETE FROM devices WHERE id = ?1", [id])?;
    Ok(StatusCode::NO_CONTENT)
}

fn matter_url(state: &AppState) -> ApiResult<&str> {
    state.matter_url.as_deref().ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "matter_server_not_configured",
            "matterjs-serverの接続先（MATTER_SERVER_URL）が設定されていません".into(),
        )
    })
}

#[allow(clippy::needless_pass_by_value)]
fn matter_unreachable(error: String) -> ApiError {
    tracing::warn!(%error, "matterjs-server unreachable");
    ApiError(
        StatusCode::BAD_GATEWAY,
        "matter_server_unreachable",
        "matterjs-serverから機器を読めませんでした".into(),
    )
}

/// A ledger device and where matterjs-server currently serves it.
#[derive(Serialize)]
struct DeviceStatus {
    id: i64,
    room_name: String,
    name: String,
    visible: bool,
    node_id: Option<u64>,
    endpoint: Option<u16>,
}

/// Locates every ledger device on matterjs-server by its vendor and serial number.
/// Devices it does not serve, or without identifiers, are not visible and need
/// registering again.
// ponytail: reads every node per request; cache with subscriptions if polled often.
async fn status(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let nodes = matter::fetch_nodes(matter_url(&state)?)
        .await
        .map_err(matter_unreachable)?;
    let db = state.db.lock().unwrap();
    let mut statement = db.prepare(&format!("{DEVICE_SELECT} ORDER BY devices.id"))?;
    let devices = statement
        .query_map([], |row| device_row(row, false))?
        .map(|device| {
            let device = device?;
            let place = device
                .vendor
                .as_deref()
                .zip(device.serial_number.as_deref())
                .and_then(|(vendor, serial)| matter::locate(&nodes, vendor, serial));
            Ok(DeviceStatus {
                id: device.id,
                room_name: device.room_name,
                name: device.name,
                visible: place.is_some(),
                node_id: place.map(|(node, _)| node),
                endpoint: place.map(|(_, endpoint)| endpoint),
            })
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "devices": devices,
    })))
}

/// A ledger device matterjs-server does not serve.
#[derive(Serialize)]
struct MissingDevice {
    id: i64,
    name: String,
    room_name: String,
}

/// A light's ledger name (else its product name) and room.
type LightName = (Option<String>, Option<String>);

/// Names `lights` from the ledger and lists the ledger devices `nodes` lack.
/// A device located on endpoint 0 names the lights of its node that are not
/// behind a bridge; a bridged device names the light on its own endpoint.
fn match_ledger(
    db: &Connection,
    nodes: &[serde_json::Value],
    lights: &[matter::Light],
) -> rusqlite::Result<(Vec<LightName>, Vec<MissingDevice>)> {
    let mut statement = db.prepare(&format!("{DEVICE_SELECT} ORDER BY devices.id"))?;
    let devices = statement
        .query_map([], |row| device_row(row, false))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut names = vec![(None, None); lights.len()];
    let mut missing = Vec::new();
    for device in devices {
        let place = device
            .vendor
            .as_deref()
            .zip(device.serial_number.as_deref())
            .and_then(|(vendor, serial)| matter::locate(nodes, vendor, serial));
        let Some((node, endpoint)) = place else {
            missing.push(MissingDevice {
                id: device.id,
                name: device.name,
                room_name: device.room_name,
            });
            continue;
        };
        for (light, name) in lights.iter().zip(&mut names) {
            let names_it = light.node_id == node
                && if light.bridged {
                    light.endpoint == endpoint
                } else {
                    endpoint == 0
                };
            if names_it && !device.name.is_empty() {
                *name = (Some(device.name.clone()), Some(device.room_name.clone()));
            }
        }
    }
    let names = names
        .into_iter()
        .zip(lights)
        .map(|((name, room), light)| (name.or_else(|| light.product.clone()), room))
        .collect();
    Ok((names, missing))
}

/// Each light with its ledger name (else its product name) and room, and `field`'s value.
fn light_entries(
    lights: &[matter::Light],
    names: Vec<LightName>,
    values: &[&str],
    field: &str,
) -> Vec<serde_json::Value> {
    lights
        .iter()
        .zip(names)
        .zip(values)
        .map(|((light, (name, room_name)), value)| {
            json!({
                "node_id": light.node_id,
                "endpoint": light.endpoint,
                "name": name,
                "room_name": room_name,
                field: value,
            })
        })
        .collect()
}

fn count(values: &[&str], value: &str) -> usize {
    values.iter().filter(|v| **v == value).count()
}

/// Counts the lights on, off and not responding, as matterjs-server last read them,
/// and the ledger devices it does not serve.
// ponytail: reads every node per request; cache with subscriptions if polled often.
async fn light_states(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let nodes = matter::fetch_nodes(matter_url(&state)?)
        .await
        .map_err(matter_unreachable)?;
    let lights = matter::lights(&nodes);
    let states: Vec<&str> = lights
        .iter()
        .map(|light| match (light.reachable, light.on) {
            (true, Some(true)) => "on",
            (true, Some(false)) => "off",
            _ => "no_response",
        })
        .collect();
    let (names, missing) = match_ledger(&state.db.lock().unwrap(), &nodes, &lights)?;
    Ok(Json(json!({
        "on": count(&states, "on"),
        "off": count(&states, "off"),
        "no_response": count(&states, "no_response"),
        "missing": missing.len(),
        "lights": light_entries(&lights, names, &states, "state"),
        "missing_devices": missing,
    })))
}

async fn lights_on(state: State<Db>) -> ApiResult<Json<serde_json::Value>> {
    switch_lights(state, true).await
}

async fn lights_off(state: State<Db>) -> ApiResult<Json<serde_json::Value>> {
    switch_lights(state, false).await
}

/// Switches every light matterjs-server serves and reports each light's result.
/// Only a light whose command was accepted counts as switched.
async fn switch_lights(State(state): State<Db>, on: bool) -> ApiResult<Json<serde_json::Value>> {
    let (nodes, results) = matter::switch(matter_url(&state)?, on)
        .await
        .map_err(matter_unreachable)?;
    let (lights, outcomes): (Vec<_>, Vec<_>) = results.into_iter().unzip();
    let outcomes: Vec<&str> = outcomes
        .into_iter()
        .map(|outcome| match outcome {
            matter::Outcome::Switched => "switched",
            matter::Outcome::NoResponse => "no_response",
            matter::Outcome::Failed => "failed",
        })
        .collect();
    let (names, missing) = match_ledger(&state.db.lock().unwrap(), &nodes, &lights)?;
    tracing::info!(
        on,
        switched = count(&outcomes, "switched"),
        lights = lights.len(),
        "lights switched"
    );
    Ok(Json(json!({
        "action": if on { "on" } else { "off" },
        "switched": count(&outcomes, "switched"),
        "no_response": count(&outcomes, "no_response"),
        "failed": count(&outcomes, "failed"),
        "missing": missing.len(),
        "lights": light_entries(&lights, names, &outcomes, "result"),
        "missing_devices": missing,
    })))
}
