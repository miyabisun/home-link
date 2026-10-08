pub mod commission;
pub mod matter;
pub mod onboarding;
pub mod schedule;

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
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
    // The user's last "all on"/"all off" and the light schedule, as JSON by key.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
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
    if version < 2 {
        // Version 2 adds each light's lowest colour temperature for the schedule.
        db.execute_batch(
            "BEGIN;
            ALTER TABLE devices ADD COLUMN min_kelvin INTEGER;
            PRAGMA user_version = 2;
            COMMIT;",
        )?;
    }
    if version < 3 {
        // Version 3 moves each light's floor to a label holding only its warm white.
        db.execute_batch(
            "BEGIN;
            CREATE TABLE labels (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                day_level INTEGER,
                night_level INTEGER,
                cool_kelvin INTEGER,
                warm_kelvin INTEGER
            );
            INSERT INTO labels (name, warm_kelvin)
                SELECT DISTINCT '夜' || min_kelvin || 'K', min_kelvin FROM devices
                WHERE min_kelvin IS NOT NULL ORDER BY min_kelvin;
            ALTER TABLE devices ADD COLUMN label_id INTEGER
                REFERENCES labels(id) ON DELETE SET NULL;
            UPDATE devices SET label_id =
                (SELECT id FROM labels WHERE warm_kelvin = devices.min_kelvin)
                WHERE min_kelvin IS NOT NULL;
            ALTER TABLE devices DROP COLUMN min_kelvin;
            PRAGMA user_version = 3;
            COMMIT;",
        )?;
    }
    Ok(db)
}

struct AppState {
    db: Mutex<Connection>,
    matter_url: Option<String>,
    /// The latest adjustments, newest first; kept in memory only.
    runs: Mutex<VecDeque<serde_json::Value>>,
    /// What the schedule last sent each light, by node and endpoint; kept in memory only.
    sent: Mutex<HashMap<(u64, u16), schedule::Target>>,
    /// How long one commissioning may take, Bluetooth discovery to Wi-Fi join.
    commission_timeout: Duration,
}

const COMMISSION_TIMEOUT: Duration = Duration::from_mins(5);

type Db = Arc<AppState>;

/// A day of runs every ten minutes.
const RUNS_KEPT: usize = 144;

/// Builds the router; `matter_url` is the matterjs-server WebSocket API used by the status API.
pub fn app(db: Connection, matter_url: Option<String>) -> Router {
    Home::new(db, matter_url).router()
}

/// The API and the light adjustment every ten minutes over one ledger.
#[derive(Clone)]
pub struct Home(Db);

impl Home {
    #[must_use]
    pub fn new(db: Connection, matter_url: Option<String>) -> Self {
        Self(Arc::new(AppState {
            db: Mutex::new(db),
            matter_url,
            runs: Mutex::new(VecDeque::new()),
            sent: Mutex::new(HashMap::new()),
            commission_timeout: COMMISSION_TIMEOUT,
        }))
    }

    /// Replaces the five-minute limit on one commissioning.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_commission_timeout(mut self, timeout: Duration) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .commission_timeout = timeout;
        self
    }

    pub fn router(&self) -> Router {
        router(self.0.clone())
    }

    /// Adjusts the lights that are on for `unix` seconds and records the run.
    pub async fn adjust(&self, unix: i64, trigger: &'static str) {
        adjust(&self.0, unix, trigger, &[]).await;
    }

    /// Adjusts the lights every ten minutes on the clock, forever.
    pub async fn every_ten_minutes(self) {
        loop {
            let wait = 600 - now().rem_euclid(600);
            tokio::time::sleep(Duration::from_secs(wait.unsigned_abs())).await;
            self.adjust(now(), "scheduled").await;
        }
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn router(state: Db) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/rooms", get(list_rooms).post(create_room))
        .route(
            "/rooms/{id}",
            axum::routing::patch(rename_room).delete(delete_room),
        )
        .route("/devices", get(list_devices).post(create_device))
        .route(
            "/devices/{id}",
            get(get_device).patch(update_device).delete(delete_device),
        )
        .route("/labels", get(list_labels).post(create_label))
        .route(
            "/labels/{id}",
            axum::routing::patch(update_label).delete(delete_label),
        )
        .route("/commission", axum::routing::post(commission))
        .route("/thread", get(thread))
        .route("/status", get(status))
        .route("/lights", get(light_states))
        .route("/lights/on", axum::routing::post(lights_on))
        .route("/lights/off", axum::routing::post(lights_off))
        .route("/lights/schedule", get(get_schedule).put(update_schedule))
        .fallback(not_found)
        // ponytail: one connection behind a global lock; a pool if writes ever contend.
        .with_state(state);

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
    /// The label whose values the schedule uses for this light.
    label_id: Option<i64>,
    label_name: Option<String>,
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
    qr_payload, vendor, serial_number, mac, label_id, labels.name
    FROM devices JOIN rooms ON rooms.id = room_id LEFT JOIN labels ON labels.id = label_id";

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
        label_id: row.get(9)?,
        label_name: row.get(10)?,
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

/// Validates the optional QR payload or manual pairing code and returns the form to store.
fn setup_code(
    qr_payload: Option<&String>,
    manual_code: Option<&String>,
) -> ApiResult<Option<String>> {
    match (qr_payload, manual_code) {
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

fn device_name(name: &str) -> ApiResult<String> {
    clean_name(name, true).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
            format!("機器名は{MAX_NAME_CHARS}文字以内で入力してください"),
        )
    })
}

fn duplicate_device() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "duplicate_qr_payload",
        "この機器は登録済みです".into(),
    )
}

fn is_registered(db: &Connection, code: &str) -> rusqlite::Result<bool> {
    Ok(registered_id(db, code)?.is_some())
}

/// The stored device sharing a passcode and short discriminator with `code`.
// ponytail: scans every device per insert; a key column if the ledger outgrows a home.
fn registered_id(db: &Connection, code: &str) -> rusqlite::Result<Option<i64>> {
    let new = onboarding::keys(code).unwrap_or_default();
    let mut statement = db.prepare("SELECT id, qr_payload FROM devices ORDER BY id")?;
    let stored = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(stored.into_iter().find_map(|(id, code)| {
        onboarding::keys(code.as_deref()?)
            .unwrap_or_default()
            .iter()
            .any(|key| new.contains(key))
            .then_some(id)
    }))
}

async fn create_device(
    State(db): State<Db>,
    Json(input): Json<DeviceInput>,
) -> ApiResult<(StatusCode, Json<Device>)> {
    let code = setup_code(input.qr_payload.as_ref(), input.manual_code.as_ref())?;
    let (serial, mac) = identifiers(&input)?;
    if code.is_none() && serial.is_none() {
        return Err(missing_setup_code());
    }
    let name = device_name(&input.name)?;
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

/// A ledger change to a device; `label_id` null removes its label.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceUpdate {
    label_id: Option<i64>,
}

async fn update_device(
    State(db): State<Db>,
    Path(id): Path<i64>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<Device>> {
    // `label_id` must be given: an empty body does not remove the label.
    let given = input
        .as_object()
        .is_some_and(|o| o.contains_key("label_id"));
    let input: DeviceUpdate = serde_json::from_value(input)
        .ok()
        .filter(|_| given)
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_device_update",
                "変更は {\"label_id\": labelのid} か、nullで外す形で送ってください".into(),
            )
        })?;
    let db = db.db.lock().unwrap();
    find_device(&db, id, false)?;
    if let Some(label) = input.label_id {
        find_label(&db, label)?;
    }
    db.execute(
        "UPDATE devices SET label_id = ?1 WHERE id = ?2",
        params![input.label_id, id],
    )?;
    Ok(Json(find_device(&db, id, false)?))
}

/// A label and its values; `null` values are the schedule's.
#[derive(Serialize)]
struct Label {
    id: i64,
    name: String,
    #[serde(flatten)]
    values: schedule::Label,
    device_count: i64,
}

const LABEL_SELECT: &str = "SELECT id, name, day_level, night_level, cool_kelvin, warm_kelvin,
    (SELECT count(*) FROM devices WHERE label_id = labels.id) FROM labels";

fn label_row(row: &rusqlite::Row) -> rusqlite::Result<Label> {
    Ok(Label {
        id: row.get(0)?,
        name: row.get(1)?,
        values: schedule::Label {
            day_level: row.get(2)?,
            night_level: row.get(3)?,
            cool_kelvin: row.get(4)?,
            warm_kelvin: row.get(5)?,
        },
        device_count: row.get(6)?,
    })
}

fn labels(db: &Connection) -> rusqlite::Result<Vec<Label>> {
    let mut statement = db.prepare(&format!("{LABEL_SELECT} ORDER BY id"))?;
    statement.query_map([], label_row)?.collect()
}

fn find_label(db: &Connection, id: i64) -> ApiResult<Label> {
    db.query_row(&format!("{LABEL_SELECT} WHERE id = ?1"), [id], label_row)
        .optional()?
        .ok_or_else(|| {
            ApiError(
                StatusCode::NOT_FOUND,
                "label_not_found",
                "labelが見つかりません".into(),
            )
        })
}

/// The name and values in `input`, checked over the schedule's current settings.
fn label_input(
    db: &Connection,
    mut input: serde_json::Value,
) -> ApiResult<(String, schedule::Label)> {
    let invalid = |message: String| ApiError(StatusCode::BAD_REQUEST, "invalid_label", message);
    let fields = input
        .as_object_mut()
        .ok_or_else(|| invalid("labelはJSONのオブジェクトで送ってください".into()))?;
    let name = fields
        .remove("name")
        .and_then(|name| clean_name(name.as_str()?, false))
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_label_name",
                format!("label名は1〜{MAX_NAME_CHARS}文字で入力してください"),
            )
        })?;
    let values: schedule::Label =
        serde_json::from_value(input).map_err(|e| invalid(e.to_string()))?;
    if let Some(message) = load::<schedule::Settings>(db, SCHEDULE)?
        .with(&values)
        .invalid()
    {
        return Err(invalid(message.into()));
    }
    Ok((name, values))
}

fn save_label(
    db: &Connection,
    id: Option<i64>,
    name: &str,
    values: &schedule::Label,
) -> ApiResult<i64> {
    let result = db.execute(
        "INSERT INTO labels (id, name, day_level, night_level, cool_kelvin, warm_kelvin)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, day_level = excluded.day_level,
            night_level = excluded.night_level, cool_kelvin = excluded.cool_kelvin,
            warm_kelvin = excluded.warm_kelvin",
        params![
            id,
            name,
            values.day_level,
            values.night_level,
            values.cool_kelvin,
            values.warm_kelvin
        ],
    );
    match result {
        Err(error) if is_unique_violation(&error) => Err(ApiError(
            StatusCode::CONFLICT,
            "duplicate_label_name",
            "同じ名前のlabelがあります".into(),
        )),
        result => {
            result?;
            Ok(id.unwrap_or_else(|| db.last_insert_rowid()))
        }
    }
}

async fn list_labels(State(db): State<Db>) -> ApiResult<Json<Vec<Label>>> {
    Ok(Json(labels(&db.db.lock().unwrap())?))
}

async fn create_label(
    State(db): State<Db>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<(StatusCode, Json<Label>)> {
    let db = db.db.lock().unwrap();
    let (name, values) = label_input(&db, input)?;
    let id = save_label(&db, None, &name, &values)?;
    Ok((StatusCode::CREATED, Json(find_label(&db, id)?)))
}

/// Merges the given fields into the label; a `null` value returns to the schedule's.
async fn update_label(
    State(db): State<Db>,
    Path(id): Path<i64>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<Label>> {
    let db = db.db.lock().unwrap();
    let label = find_label(&db, id)?;
    let mut merged = json!(label.values);
    merged["name"] = json!(label.name);
    if let Some(fields) = input.as_object() {
        for (key, value) in fields {
            merged[key] = value.clone();
        }
    } else {
        merged = input;
    }
    let (name, values) = label_input(&db, merged)?;
    save_label(&db, Some(id), &name, &values)?;
    Ok(Json(find_label(&db, id)?))
}

/// Deletes the label; its lights return to the schedule's values.
async fn delete_label(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let db = db.db.lock().unwrap();
    find_label(&db, id)?;
    db.execute("DELETE FROM labels WHERE id = ?1", [id])?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_device(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let db = db.db.lock().unwrap();
    find_device(&db, id, false)?;
    db.execute("DELETE FROM devices WHERE id = ?1", [id])?;
    Ok(StatusCode::NO_CONTENT)
}

/// A new device to commission over the phone's Bluetooth and record. The Wi-Fi
/// password is handed to matterjs-server and kept nowhere else.
#[derive(Deserialize)]
struct CommissionInput {
    room_id: i64,
    qr_payload: Option<String>,
    manual_code: Option<String>,
    #[serde(default)]
    name: String,
    /// `wifi` (the default) or `thread`.
    network: Option<String>,
    #[serde(default)]
    wifi_ssid: String,
    #[serde(default)]
    wifi_password: String,
}

/// Has matterjs-server commission the device over the BLE proxy the phone holds
/// open, onto the given Wi-Fi, then records it: a device already in the ledger
/// (by identifiers, else by code) keeps its entry and gains missing identifiers.
async fn commission(
    State(state): State<Db>,
    Json(input): Json<CommissionInput>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let code =
        setup_code(input.qr_payload.as_ref(), input.manual_code.as_ref())?.ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "missing_setup_code",
                "QRコードか手動ペアリングコードのどちらか一方を送ってください".into(),
            )
        })?;
    let network = match input.network.as_deref() {
        None | Some("wifi") => commission::Network::Wifi {
            ssid: &input.wifi_ssid,
            password: &input.wifi_password,
        },
        Some("thread") => commission::Network::Thread,
        Some(_) => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_network",
                "networkは wifi か thread で指定してください".into(),
            ));
        }
    };
    // SSIDs are up to 32 bytes; WPA passphrases 8–63 characters or 64 hex digits.
    if let commission::Network::Wifi { ssid, password } = network
        && (!(1..=32).contains(&ssid.len()) || !(1..=64).contains(&password.len()))
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_wifi",
            "Wi-Fiの名前（32バイト以内）とパスワード（64文字以内）を送ってください".into(),
        ));
    }
    let name = device_name(&input.name)?;
    find_room(&state.db.lock().unwrap(), input.room_id)?.ok_or_else(room_not_found)?;
    let url = matter_url(&state)?;
    let commissioned = commission::commission(url, &code, &network, state.commission_timeout)
        .await
        .map_err(|error| match error {
            commission::Error::Unreachable(error) => matter_unreachable(error),
            commission::Error::Failed(failure) => commission_failed(failure, &input.wifi_ssid),
        })?;
    let db = state.db.lock().unwrap();
    let (registered, device) = record(&db, input.room_id, &name, &code, &commissioned)?;
    tracing::info!(
        node_id = commissioned.node_id,
        registered,
        device = device.id,
        "device commissioned"
    );
    Ok((
        if registered {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({
            "node_id": commissioned.node_id,
            "registered": registered,
            "device": device,
        })),
    ))
}

fn commission_failed(failure: commission::Failure, ssid: &str) -> ApiError {
    use commission::Failure;
    tracing::warn!(?failure, "commissioning failed");
    let (status, code, message) = match failure {
        Failure::NotFound => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "device_not_found",
            "ペアリング待ちの機器がBluetoothで見つかりませんでした".to_owned(),
        ),
        Failure::WrongCode => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong_code",
            "機器がコードを受け付けませんでした".to_owned(),
        ),
        Failure::Wifi => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "wifi_failed",
            format!("機器がWi-Fi「{ssid}」に接続できませんでした"),
        ),
        Failure::Thread => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "thread_failed",
            "機器がThread網に参加できませんでした".to_owned(),
        ),
        Failure::ThreadNotReady => (
            StatusCode::SERVICE_UNAVAILABLE,
            "thread_not_ready",
            "Thread網が未準備です".to_owned(),
        ),
        Failure::Timeout => (
            StatusCode::GATEWAY_TIMEOUT,
            "commission_timeout",
            "時間内に登録が終わりませんでした".to_owned(),
        ),
        Failure::BluetoothDisabled => (
            StatusCode::SERVICE_UNAVAILABLE,
            "bluetooth_unavailable",
            "matterjs-serverでBluetoothが有効になっていません".to_owned(),
        ),
        Failure::Other(details) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "commission_failed",
            format!("登録できませんでした: {details}"),
        ),
    };
    ApiError(status, code, message)
}

/// Whether Thread devices can be commissioned: matterjs-server holds a dataset.
async fn thread(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let ready = commission::thread_ready(matter_url(&state)?, THREAD_TIMEOUT)
        .await
        .map_err(matter_unreachable)?;
    Ok(Json(json!({ "ready": ready })))
}

const THREAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Records a commissioned device; `false` with the existing entry when the
/// ledger already holds it.
fn record(
    db: &Connection,
    room_id: i64,
    name: &str,
    code: &str,
    commissioned: &commission::Commissioned,
) -> ApiResult<(bool, Device)> {
    let (vendor, serial) = commissioned.identity.clone().unzip();
    let mac = &commissioned.mac;
    let by_identifier: Option<i64> = db
        .query_row(
            "SELECT id FROM devices WHERE (vendor = ?1 AND serial_number = ?2) OR mac = ?3
             ORDER BY id LIMIT 1",
            params![vendor, serial, mac],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = by_identifier.or(registered_id(db, code)?) {
        db.execute(
            "UPDATE devices SET vendor = coalesce(vendor, ?1),
                serial_number = coalesce(serial_number, ?2), mac = coalesce(mac, ?3)
             WHERE id = ?4",
            params![vendor, serial, mac, id],
        )?;
        return Ok((false, find_device(db, id, false)?));
    }
    db.execute(
        "INSERT INTO devices (room_id, qr_payload, vendor, serial_number, mac, name)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![room_id, code, vendor, serial, mac, name],
    )?;
    Ok((true, find_device(db, db.last_insert_rowid(), false)?))
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
            let place = place(&device, &nodes);
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

fn ledger(db: &Connection) -> rusqlite::Result<Vec<Device>> {
    let mut statement = db.prepare(&format!("{DEVICE_SELECT} ORDER BY devices.id"))?;
    statement
        .query_map([], |row| device_row(row, false))?
        .collect()
}

fn place(device: &Device, nodes: &[serde_json::Value]) -> Option<(u64, u16)> {
    device
        .vendor
        .as_deref()
        .zip(device.serial_number.as_deref())
        .and_then(|(vendor, serial)| matter::locate(nodes, vendor, serial))
}

/// The ledger devices that are `light`, latest first: a device located on
/// endpoint 0 is the lights of its node that are not behind a bridge; a bridged
/// device the light on its own endpoint.
fn devices_of<'a>(
    devices: &'a [Device],
    nodes: &'a [serde_json::Value],
    light: &'a matter::Light,
) -> impl Iterator<Item = &'a Device> {
    devices.iter().rev().filter(move |device| {
        place(device, nodes).is_some_and(|(node, endpoint)| {
            light.node_id == node
                && if light.bridged {
                    light.endpoint == endpoint
                } else {
                    endpoint == 0
                }
        })
    })
}

/// Names `lights` from the ledger and lists the ledger devices `nodes` lack.
fn match_ledger(
    devices: &[Device],
    nodes: &[serde_json::Value],
    lights: &[matter::Light],
) -> (Vec<LightName>, Vec<MissingDevice>) {
    let names = lights
        .iter()
        .map(|light| {
            devices_of(devices, nodes, light)
                .find(|device| !device.name.is_empty())
                .map_or((light.product.clone(), None), |device| {
                    (Some(device.name.clone()), Some(device.room_name.clone()))
                })
        })
        .collect();
    let missing = devices
        .iter()
        .filter(|device| place(device, nodes).is_none())
        .map(|device| MissingDevice {
            id: device.id,
            name: device.name.clone(),
            room_name: device.room_name.clone(),
        })
        .collect();
    (names, missing)
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
    let (names, missing) = match_ledger(&ledger(&state.db.lock().unwrap())?, &nodes, &lights);
    Ok(Json(json!({
        "on": count(&states, "on"),
        "off": count(&states, "off"),
        "no_response": count(&states, "no_response"),
        "missing": missing.len(),
        "lights": light_entries(&lights, names, &states, "state"),
        "missing_devices": missing,
    })))
}

/// Switches every light on and, when the schedule is on, brings them to the current values.
async fn lights_on(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let (response, switched) = switch_lights(&state, true).await?;
    if load::<schedule::Settings>(&state.db.lock().unwrap(), SCHEDULE)?.enabled {
        adjust(&state, now(), "lights_on", &switched).await;
    }
    Ok(response)
}

async fn lights_off(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    Ok(switch_lights(&state, false).await?.0)
}

/// Records "all on" or "all off" as the user's intent, switches every light
/// matterjs-server serves and reports each light's result, with the node and
/// endpoint of the switched ones.
/// Only a light whose command was accepted counts as switched.
async fn switch_lights(
    state: &AppState,
    on: bool,
) -> ApiResult<(Json<serde_json::Value>, Vec<(u64, u16)>)> {
    let url = matter_url(state)?;
    let intent = Intent {
        action: if on { "on" } else { "off" }.into(),
        at: schedule::timestamp(now()),
    };
    store(&state.db.lock().unwrap(), INTENT, &intent)?;
    let (nodes, results) = matter::switch(url, on).await.map_err(matter_unreachable)?;
    let (lights, outcomes): (Vec<_>, Vec<_>) = results.into_iter().unzip();
    let switched = lights
        .iter()
        .zip(&outcomes)
        .filter(|(_, outcome)| **outcome == matter::Outcome::Switched)
        .map(|(light, _)| (light.node_id, light.endpoint))
        .collect();
    let outcomes: Vec<&str> = outcomes
        .into_iter()
        .map(|outcome| match outcome {
            matter::Outcome::Switched => "switched",
            matter::Outcome::NoResponse => "no_response",
            matter::Outcome::Failed => "failed",
        })
        .collect();
    let (names, missing) = match_ledger(&ledger(&state.db.lock().unwrap())?, &nodes, &lights);
    tracing::info!(
        on,
        switched = count(&outcomes, "switched"),
        lights = lights.len(),
        "lights switched"
    );
    let response = Json(json!({
        "action": if on { "on" } else { "off" },
        "switched": count(&outcomes, "switched"),
        "no_response": count(&outcomes, "no_response"),
        "failed": count(&outcomes, "failed"),
        "missing": missing.len(),
        "lights": light_entries(&lights, names, &outcomes, "result"),
        "missing_devices": missing,
    }));
    Ok((response, switched))
}

const INTENT: &str = "lights_intent";
const SCHEDULE: &str = "light_schedule";

/// The last "all on" or "all off" pressed: the only light state kept across restarts.
#[derive(Serialize, Deserialize)]
struct Intent {
    action: String,
    at: String,
}

fn load<T: DeserializeOwned + Default>(db: &Connection, key: &str) -> rusqlite::Result<T> {
    let value: Option<String> = db
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(value
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

fn store<T: Serialize>(db: &Connection, key: &str, value: &T) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, serde_json::to_string(value).unwrap_or_default()],
    )?;
    Ok(())
}

/// Adjusts every light that is on to the values for `unix`, its label's in
/// place of the schedule's, and records the run: nothing is written while
/// "all off" is in force or the schedule is disabled, nor what was already sent
/// unless `switched_on` holds the light: just switched on, it gets all values.
async fn adjust(state: &AppState, unix: i64, trigger: &'static str, switched_on: &[(u64, u16)]) {
    let loaded = {
        let db = state.db.lock().unwrap();
        load::<schedule::Settings>(&db, SCHEDULE).and_then(|settings| {
            Ok((
                settings,
                load::<Option<Intent>>(&db, INTENT)?,
                ledger(&db)?,
                labels(&db)?,
            ))
        })
    };
    let (day, minute) = schedule::local(unix);
    let mut run = json!({ "at": schedule::timestamp(unix), "trigger": trigger, "commands": 0 });
    let result = match (&loaded, &state.matter_url) {
        (Err(error), _) => Err(error.to_string()),
        (_, None) => Err("MATTER_SERVER_URL is not set".into()),
        (Ok((settings, intent, devices, labels)), Some(url)) => {
            let sun = schedule::sun_times(day, settings.latitude, settings.longitude);
            let goal = schedule::target(minute, sun, settings);
            let all_off = intent.as_ref().is_some_and(|i| i.action == "off");
            run["sunrise"] = json!(schedule::clock(sun.0));
            run["sunset"] = json!(schedule::clock(sun.1));
            run["level"] = json!(goal.0);
            run["kelvin"] = json!(goal.1);
            let sent = state.sent.lock().unwrap().clone();
            let goal_of = |nodes: &[serde_json::Value], light: &matter::Light| {
                let label = devices_of(devices, nodes, light)
                    .find_map(|d| labels.iter().find(|l| Some(l.id) == d.label_id));
                let goal = label.map_or(goal, |label| {
                    schedule::target(minute, sun, &settings.with(&label.values))
                });
                (label.map(|l| l.name.clone()), goal)
            };
            matter::tune(url, switched_on, |nodes, light| {
                let key = (light.node_id, light.endpoint);
                let last = sent
                    .get(&key)
                    .copied()
                    .filter(|_| !switched_on.contains(&key));
                schedule::plan(light, all_off, settings, goal_of(nodes, light).1, last)
            })
            .await
            .map(|tuned| {
                let goals: Vec<_> = tuned
                    .lights
                    .iter()
                    .map(|(light, _)| goal_of(&tuned.nodes, light))
                    .collect();
                (tuned, goals)
            })
        }
    };
    match result {
        Ok((tuned, goals)) => {
            let lights: Vec<_> = tuned.lights.iter().map(|(l, _)| l.clone()).collect();
            let devices = loaded.map(|(_, _, devices, _)| devices).unwrap_or_default();
            let (names, _) = match_ledger(&devices, &tuned.nodes, &lights);
            let mut memory = state.sent.lock().unwrap();
            let entries: Vec<_> = tuned
                .lights
                .iter()
                .zip(names)
                .zip(goals)
                .map(|(((light, decision), (name, room_name)), (label, goal))| {
                    let mut entry = json!({
                        "node_id": light.node_id,
                        "endpoint": light.endpoint,
                        "name": name,
                        "room_name": room_name,
                        "label": label,
                        "target_level": goal.0,
                        "target_kelvin": goal.1,
                    });
                    entry["decision"] = decide(&mut memory, light, *decision, &mut entry);
                    entry
                })
                .collect();
            let sent = entries.iter().filter(|e| e["decision"] == "sent").count();
            tracing::info!(trigger, commands = tuned.commands, sent, "lights adjusted");
            run["commands"] = json!(tuned.commands);
            run["lights"] = json!(entries);
        }
        Err(error) => {
            tracing::warn!(trigger, %error, "light adjustment failed");
            run["error"] = json!(error);
            run["lights"] = json!([]);
        }
    }
    let mut runs = state.runs.lock().unwrap();
    runs.push_front(run);
    runs.truncate(RUNS_KEPT);
}

/// `decision` for the run's `entry`, remembering what was sent `light`.
fn decide(
    memory: &mut HashMap<(u64, u16), schedule::Target>,
    light: &matter::Light,
    decision: matter::Decision,
    entry: &mut serde_json::Value,
) -> serde_json::Value {
    match decision {
        matter::Decision::Skipped(skip) => json!(skip),
        matter::Decision::Off => json!("off"),
        matter::Decision::NoResponse => json!("no_response"),
        matter::Decision::Failed => json!("failed"),
        matter::Decision::Sent(target) => {
            let last = memory
                .entry((light.node_id, light.endpoint))
                .or_insert(target);
            last.level = target.level.or(last.level);
            last.mireds = target.mireds.or(last.mireds);
            entry["level"] = json!(target.level);
            entry["mireds"] = json!(target.mireds);
            json!("sent")
        }
    }
}

/// A ledger device with a label.
#[derive(Serialize)]
struct LabelledDevice {
    id: i64,
    name: String,
    room_name: String,
}

/// The schedule's settings, each label's values and devices, the user's last
/// "all on"/"all off" and the latest runs.
async fn get_schedule(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let (settings, intent, labels) = {
        let db = state.db.lock().unwrap();
        let devices = ledger(&db)?;
        let labels: Vec<_> = labels(&db)?
            .into_iter()
            .map(|label| {
                let mut entry = json!(label.values);
                entry["id"] = json!(label.id);
                entry["name"] = json!(label.name);
                entry["devices"] = json!(
                    devices
                        .iter()
                        .filter(|d| d.label_id == Some(label.id))
                        .map(|d| LabelledDevice {
                            id: d.id,
                            name: d.name.clone(),
                            room_name: d.room_name.clone(),
                        })
                        .collect::<Vec<_>>()
                );
                entry
            })
            .collect();
        (
            load::<schedule::Settings>(&db, SCHEDULE)?,
            load::<Option<Intent>>(&db, INTENT)?,
            labels,
        )
    };
    let runs: Vec<_> = state.runs.lock().unwrap().iter().cloned().collect();
    Ok(Json(json!({
        "settings": settings,
        "labels": labels,
        "intent": intent,
        "runs": runs,
    })))
}

/// Merges the given fields into the schedule's settings.
async fn update_schedule(
    State(state): State<Db>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let invalid = |message: String| ApiError(StatusCode::BAD_REQUEST, "invalid_schedule", message);
    {
        let db = state.db.lock().unwrap();
        let mut settings = json!(load::<schedule::Settings>(&db, SCHEDULE)?);
        let fields = input
            .as_object()
            .ok_or_else(|| invalid("設定はJSONのオブジェクトで送ってください".into()))?;
        for (key, value) in fields {
            settings[key] = value.clone();
        }
        let settings: schedule::Settings =
            serde_json::from_value(settings).map_err(|e| invalid(e.to_string()))?;
        if let Some(message) = settings.invalid() {
            return Err(invalid(message.into()));
        }
        store(&db, SCHEDULE, &settings)?;
    }
    get_schedule(State(state)).await
}
