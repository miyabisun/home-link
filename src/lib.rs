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

// ponytail: one connection behind a global lock; a pool if writes ever contend.
type Db = Arc<Mutex<Connection>>;

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
    Ok(db)
}

pub fn app(db: Connection) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/rooms", get(list_rooms).post(create_room))
        .route(
            "/rooms/{id}",
            axum::routing::patch(rename_room).delete(delete_room),
        )
        .route("/devices", get(list_devices).post(create_device))
        .route("/devices/{id}", get(get_device).delete(delete_device))
        .fallback(not_found)
        .with_state(Arc::new(Mutex::new(db)));

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
    Json(json!({ "status": "ok" }))
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
    let db = db.lock().unwrap();
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
    let db = db.lock().unwrap();
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
    let db = db.lock().unwrap();
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
    let db = db.lock().unwrap();
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
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    qr_payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_code: Option<String>,
}

/// A device to register with exactly one of its QR payload or manual pairing code.
#[derive(Deserialize)]
struct DeviceInput {
    room_id: i64,
    qr_payload: Option<String>,
    manual_code: Option<String>,
    #[serde(default)]
    name: String,
}

const DEVICE_SELECT: &str = "SELECT devices.id, room_id, rooms.name, devices.name, created_at,
    qr_payload FROM devices JOIN rooms ON rooms.id = room_id";

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
    let db = db.lock().unwrap();
    let mut statement = db.prepare(&format!("{DEVICE_SELECT} ORDER BY devices.id"))?;
    let devices = statement
        .query_map([], |row| device_row(row, false))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Json(devices))
}

async fn get_device(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<Json<Device>> {
    Ok(Json(find_device(&db.lock().unwrap(), id, true)?))
}

/// Validates the one setup code of `input` and returns the form to store.
fn setup_code(input: &DeviceInput) -> ApiResult<String> {
    match (&input.qr_payload, &input.manual_code) {
        (Some(payload), None) => {
            let payload = payload.trim();
            (payload.len() <= MAX_QR_CHARS && onboarding::is_valid(payload))
                .then(|| payload.to_owned())
                .ok_or_else(|| {
                    ApiError(
                        StatusCode::BAD_REQUEST,
                        "invalid_qr_payload",
                        "MatterのQRコード（MT:で始まる）ではありません".into(),
                    )
                })
        }
        (None, Some(code)) => onboarding::normalize_manual(code.trim()).ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_manual_code",
                "Matterの手動ペアリングコード（11桁の数字）として正しくありません".into(),
            )
        }),
        _ => Err(ApiError(
            StatusCode::BAD_REQUEST,
            "missing_setup_code",
            "QRコードか手動ペアリングコードのどちらか一方を送ってください".into(),
        )),
    }
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
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(stored
        .iter()
        .flat_map(|code| onboarding::keys(code).unwrap_or_default())
        .any(|key| new.contains(&key)))
}

async fn create_device(
    State(db): State<Db>,
    Json(input): Json<DeviceInput>,
) -> ApiResult<(StatusCode, Json<Device>)> {
    let code = setup_code(&input)?;
    let name = clean_name(&input.name, true).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
            format!("機器名は{MAX_NAME_CHARS}文字以内で入力してください"),
        )
    })?;
    let db = db.lock().unwrap();
    find_room(&db, input.room_id)?.ok_or_else(room_not_found)?;
    if is_registered(&db, &code)? {
        return Err(duplicate_device());
    }
    match db.execute(
        "INSERT INTO devices (room_id, qr_payload, name) VALUES (?1, ?2, ?3)",
        params![input.room_id, code, name],
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
    let db = db.lock().unwrap();
    find_device(&db, id, false)?;
    db.execute("DELETE FROM devices WHERE id = ?1", [id])?;
    Ok(StatusCode::NO_CONTENT)
}
