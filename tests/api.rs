use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

const QR: &str = "MT:Y.K9042C00KA0648G00";
// The same device as `QR` (passcode 20202021, discriminator 3840).
const MANUAL: &str = "34970112332";
// Another device: passcode 20202022.
const OTHER_QR: &str = "MT:Y.K9042C000O0648G00";

fn app() -> Router {
    home_link::app(home_link::open_db(":memory:").unwrap(), None)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn create_room(app: &Router, name: &str) -> i64 {
    let (status, room) = call(app, "POST", "/api/rooms", Some(json!({ "name": name }))).await;
    assert_eq!(status, StatusCode::CREATED, "{room}");
    room["id"].as_i64().unwrap()
}

#[tokio::test]
async fn health_endpoints_respond() {
    let app = app();
    let (status, body) = call(&app, "GET", "/api/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") })
    );
    let (status, _) = call(&app, "GET", "/api/missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rooms_support_create_list_rename_and_delete() {
    let app = app();
    let living = create_room(&app, " リビング ").await;
    let bedroom = create_room(&app, "寝室").await;

    let (status, rooms) = call(&app, "GET", "/api/rooms", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        rooms,
        json!([
            { "id": living, "name": "リビング", "device_count": 0 },
            { "id": bedroom, "name": "寝室", "device_count": 0 },
        ])
    );

    let uri = format!("/api/rooms/{living}");
    let (status, room) = call(&app, "PATCH", &uri, Some(json!({ "name": "居間" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        room,
        json!({ "id": living, "name": "居間", "device_count": 0 })
    );

    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, rooms) = call(&app, "GET", "/api/rooms", None).await;
    assert_eq!(rooms.as_array().unwrap().len(), 1);

    let (status, error) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"], "room_not_found");
    let (status, _) = call(&app, "PATCH", &uri, Some(json!({ "name": "x" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn room_names_must_be_present_and_unique() {
    let app = app();
    let bedroom = create_room(&app, "寝室").await;
    for name in ["", "   ", &"あ".repeat(101)] {
        let (status, error) = call(&app, "POST", "/api/rooms", Some(json!({ "name": name }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?}");
        assert_eq!(error["error"], "invalid_room_name");
    }
    let (status, error) = call(&app, "POST", "/api/rooms", Some(json!({ "name": "寝室" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"], "duplicate_room_name");

    let other = create_room(&app, "書斎").await;
    let uri = format!("/api/rooms/{other}");
    let (status, _) = call(&app, "PATCH", &uri, Some(json!({ "name": "寝室" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let uri = format!("/api/rooms/{bedroom}");
    let (status, _) = call(&app, "PATCH", &uri, Some(json!({ "name": "寝室" }))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn rooms_with_devices_cannot_be_deleted() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let device = json!({ "room_id": room, "qr_payload": QR, "name": "" });
    let (status, created) = call(&app, "POST", "/api/devices", Some(device)).await;
    assert_eq!(status, StatusCode::CREATED);

    let uri = format!("/api/rooms/{room}");
    let (status, error) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"], "room_has_devices");
    assert!(error["message"].as_str().unwrap().contains('1'));

    let (_, rooms) = call(&app, "GET", "/api/rooms", None).await;
    assert_eq!(rooms[0]["device_count"], 1);

    let device_uri = format!("/api/devices/{}", created["id"]);
    let (status, _) = call(&app, "DELETE", &device_uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn devices_register_with_optional_name_and_hide_payload_in_list() {
    let app = app();
    let room = create_room(&app, "寝室").await;

    let (status, unnamed) = call(
        &app,
        "POST",
        "/api/devices",
        Some(json!({ "room_id": room, "qr_payload": QR })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{unnamed}");
    assert_eq!(unnamed["name"], "");
    assert_eq!(unnamed["room_id"], room);
    assert_eq!(unnamed["room_name"], "寝室");
    assert!(unnamed.get("qr_payload").is_none());

    let second = format!("{OTHER_QR}*{}", &OTHER_QR[3..]);
    let (status, named) = call(
        &app,
        "POST",
        "/api/devices",
        Some(json!({ "room_id": room, "qr_payload": second, "name": " 天井灯 " })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(named["name"], "天井灯");

    let (status, devices) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(devices.as_array().unwrap().len(), 2);
    assert!(!devices.to_string().contains("Y.K9042C00"));

    let uri = format!("/api/devices/{}", named["id"]);
    let (status, detail) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["qr_payload"], second);
    assert_eq!(detail["name"], "天井灯");

    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, error) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"], "device_not_found");
}

#[tokio::test]
async fn device_registration_rejects_invalid_input_and_duplicates() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let post = |body: Value| {
        let app = app.clone();
        async move { call(&app, "POST", "/api/devices", Some(body)).await }
    };

    let (status, error) = post(json!({ "room_id": room, "qr_payload": "MT:abc" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_qr_payload");
    assert!(!error.to_string().contains("MT:abc"));

    let (status, error) = post(json!({ "room_id": room + 99, "qr_payload": QR })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"], "room_not_found");

    let long_name = "あ".repeat(101);
    let (status, error) =
        post(json!({ "room_id": room, "qr_payload": QR, "name": long_name })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_device_name");

    let (status, _) = post(json!({ "room_id": room, "qr_payload": QR })).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, error) = post(json!({ "room_id": room, "qr_payload": format!(" {QR}\n") })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"], "duplicate_qr_payload");

    let (status, _) = post(json!({ "qr_payload": QR })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn devices_register_with_a_manual_pairing_code_and_hide_it_in_list() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let body = json!({ "room_id": room, "manual_code": " 3497-011 2332 ", "name": "電球" });
    let (status, created) = call(&app, "POST", "/api/devices", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"], "電球");
    assert!(created.get("manual_code").is_none());

    let (_, devices) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(devices.as_array().unwrap().len(), 1);
    assert!(!devices.to_string().contains("3497"));

    let uri = format!("/api/devices/{}", created["id"]);
    let (status, detail) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["manual_code"], MANUAL);
    assert!(detail.get("qr_payload").is_none());
}

#[tokio::test]
async fn qr_and_manual_codes_of_one_device_are_duplicates() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let post = |body: Value| {
        let app = app.clone();
        async move { call(&app, "POST", "/api/devices", Some(body)).await }
    };

    let (status, manual) = post(json!({ "room_id": room, "manual_code": MANUAL })).await;
    assert_eq!(status, StatusCode::CREATED);
    for body in [
        json!({ "room_id": room, "manual_code": "3497 011 2332" }),
        json!({ "room_id": room, "qr_payload": QR }),
        json!({ "room_id": room, "qr_payload": format!("{OTHER_QR}*{}", &QR[3..]) }),
    ] {
        let (status, error) = post(body.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(error["error"], "duplicate_qr_payload");
    }

    let uri = format!("/api/devices/{}", manual["id"]);
    call(&app, "DELETE", &uri, None).await;
    let (status, _) = post(json!({ "room_id": room, "qr_payload": QR })).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = post(json!({ "room_id": room, "manual_code": MANUAL })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = post(json!({ "room_id": room, "qr_payload": OTHER_QR })).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn device_registration_needs_exactly_one_valid_code() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let post = |body: Value| {
        let app = app.clone();
        async move { call(&app, "POST", "/api/devices", Some(body)).await }
    };

    let (status, error) = post(json!({ "room_id": room, "manual_code": "34970112331" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_manual_code");
    assert!(!error.to_string().contains("3497"));

    // A manual code is not accepted as a QR payload.
    let (status, error) = post(json!({ "room_id": room, "qr_payload": MANUAL })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_qr_payload");

    for body in [
        json!({ "room_id": room }),
        json!({ "room_id": room, "qr_payload": QR, "manual_code": MANUAL }),
    ] {
        let (status, error) = post(body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(error["error"], "missing_setup_code");
    }
}

const TAPO: &str = "CCBABDE0C244";

#[tokio::test]
async fn devices_register_with_identifiers_and_without_a_code() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let body = json!({
        "room_id": room,
        "vendor": " Tapo ",
        "serial_number": " CCBABDE0C244 ",
        "mac": "cc:ba:bd:e0:c2:44",
        "name": "天井灯",
    });
    let (status, created) = call(&app, "POST", "/api/devices", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["vendor"], "Tapo");
    assert_eq!(created["serial_number"], TAPO);
    assert_eq!(created["mac"], "CCBABDE0C244");

    let (_, devices) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(devices[0]["serial_number"], TAPO);
    let uri = format!("/api/devices/{}", created["id"]);
    let (_, detail) = call(&app, "GET", &uri, None).await;
    assert!(detail.get("qr_payload").is_none() && detail.get("manual_code").is_none());

    // A code and identifiers may be registered together.
    let body = json!({
        "room_id": room, "qr_payload": QR, "vendor": "Aqara", "serial_number": "54ef441001372c4c",
    });
    let (status, both) = call(&app, "POST", "/api/devices", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{both}");
    assert_eq!(both["vendor"], "Aqara");
    assert!(both["mac"].is_null());
}

#[tokio::test]
async fn device_identifiers_are_validated_and_unique() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let post = |body: Value| {
        let app = app.clone();
        async move { call(&app, "POST", "/api/devices", Some(body)).await }
    };

    let tapo = json!({ "room_id": room, "vendor": "Tapo", "serial_number": TAPO, "mac": TAPO });
    assert_eq!(post(tapo.clone()).await.0, StatusCode::CREATED);
    for body in [
        json!({ "room_id": room, "vendor": "Tapo", "serial_number": TAPO }),
        json!({ "room_id": room, "vendor": "Tapo", "serial_number": "OTHER", "mac": "CC-BA-BD-E0-C2-44" }),
    ] {
        let (status, error) = post(body.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(error["error"], "duplicate_identifier");
    }
    let other_vendor = json!({ "room_id": room, "vendor": "Aqara", "serial_number": TAPO });
    assert_eq!(post(other_vendor).await.0, StatusCode::CREATED);

    for body in [
        json!({ "room_id": room, "vendor": "Tapo" }),
        json!({ "room_id": room, "serial_number": "S1" }),
        json!({ "room_id": room, "vendor": " ", "serial_number": "S1" }),
        json!({ "room_id": room, "vendor": "Tapo", "serial_number": "S".repeat(101) }),
    ] {
        let (status, error) = post(body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(error["error"], "invalid_identifier");
    }
    for mac in ["CCBABDE0C24", "CCBABDE0C24G", "CC:BA:BD:E0:C2:44:00"] {
        let body = json!({ "room_id": room, "vendor": "Tapo", "serial_number": "S2", "mac": mac });
        let (status, error) = post(body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{mac}");
        assert_eq!(error["error"], "invalid_mac");
    }

    // A MAC alone does not identify a device without a code.
    let (status, error) = post(json!({ "room_id": room, "mac": "001122334455" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "missing_setup_code");
}

#[test]
fn opening_a_ledger_from_the_code_only_schema_keeps_its_devices() {
    let path = std::env::temp_dir().join(format!("home-link-migrate-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch(
        "CREATE TABLE rooms (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
        CREATE TABLE devices (
            id INTEGER PRIMARY KEY,
            room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
            qr_payload TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        INSERT INTO rooms (name) VALUES ('寝室');
        INSERT INTO devices (room_id, qr_payload, name) VALUES (1, '34970112332', '電球');",
    )
    .unwrap();
    drop(old);

    for round in 0..2 {
        let db = home_link::open_db(path.to_str().unwrap()).unwrap();
        let row: (String, String, Option<String>, Option<u16>) = db
            .query_row(
                "SELECT qr_payload, name, serial_number, min_kelvin FROM devices WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, ("34970112332".into(), "電球".into(), None, None));
        db.execute(
            "INSERT INTO devices (room_id, vendor, serial_number) VALUES (1, 'Tapo', ?1)",
            [format!("S{round}")],
        )
        .unwrap();
    }
    let _ = std::fs::remove_file(&path);
}

/// Serves one `get_nodes` result the way matterjs-server does: a server
/// info message on connect, then the result under the request's message id.
async fn fake_matter_server(nodes: Value) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let info = json!({ "fabric_id": 1, "schema_version": 13 });
            ws.send(Message::text(info.to_string())).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let command: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(command["command"], "get_nodes");
                let event = json!({ "event": "node_updated", "data": {} });
                ws.send(Message::text(event.to_string())).await.unwrap();
                let reply = json!({ "message_id": command["message_id"], "result": nodes });
                ws.send(Message::text(reply.to_string())).await.unwrap();
            }
        }
    });
    addr
}

#[tokio::test]
async fn status_locates_ledger_devices_on_the_matter_server() {
    let nodes = json!([
        { "node_id": 1, "attributes": { "0/40/1": "Aqara", "0/40/15": "hub", "3/57/1": "Aqara", "3/57/15": "54ef441001372c4c" }},
        { "node_id": 5, "attributes": { "0/40/1": "Tapo", "0/40/15": TAPO }},
    ]);
    let addr = fake_matter_server(nodes).await;
    let db = home_link::open_db(":memory:").unwrap();
    let app = home_link::app(db, Some(format!("ws://{addr}/ws")));
    let room = create_room(&app, "寝室").await;
    for body in [
        json!({ "room_id": room, "vendor": "Tapo", "serial_number": TAPO, "name": "A" }),
        json!({ "room_id": room, "vendor": "Aqara", "serial_number": "54ef441001372c4c", "name": "B" }),
        json!({ "room_id": room, "vendor": "Tapo", "serial_number": "CCBABDE0FFFF", "name": "C" }),
        json!({ "room_id": room, "manual_code": MANUAL, "name": "D" }),
    ] {
        assert_eq!(
            call(&app, "POST", "/api/devices", Some(body)).await.0,
            StatusCode::CREATED
        );
    }

    let (status, body) = call(&app, "GET", "/api/status", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    let seen: Vec<_> = body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["name"].clone(),
                d["room_name"].clone(),
                d["visible"].clone(),
                d["node_id"].clone(),
                d["endpoint"].clone(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (json!("A"), json!("寝室"), json!(true), json!(5), json!(0)),
            (json!("B"), json!("寝室"), json!(true), json!(1), json!(3)),
            (
                json!("C"),
                json!("寝室"),
                json!(false),
                Value::Null,
                Value::Null
            ),
            (
                json!("D"),
                json!("寝室"),
                json!(false),
                Value::Null,
                Value::Null
            ),
        ]
    );
    assert!(!body.to_string().contains("3497"));
}

#[tokio::test]
async fn status_reports_an_unset_or_unreachable_matter_server() {
    let (status, error) = call(&app(), "GET", "/api/status", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"], "matter_server_not_configured");

    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let db = home_link::open_db(":memory:").unwrap();
    let app = home_link::app(db, Some(format!("ws://{closed}/ws")));
    let (status, error) = call(&app, "GET", "/api/status", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(error["error"], "matter_server_unreachable");
}

/// How the fake matterjs-server answers `device_command`.
#[derive(Clone, Default)]
struct Fake {
    /// `(node_id, endpoint)` answered with an error.
    failing: Vec<(u64, u64)>,
    /// `(node_id, endpoint)` never answered.
    silent: Vec<(u64, u64)>,
    /// Close the first connection when its first command arrives.
    drop_first: bool,
    /// `(node_id, endpoint)` read as off although `get_nodes` serves it on.
    read_off: Vec<(u64, u64)>,
    /// Off is accepted but the lights stay on, as if switched on again by other means.
    ignore_off: bool,
}

/// Serves `get_nodes` and On/Off `device_command` the way matterjs-server does,
/// applying each command to the served On/Off attribute and recording it.
async fn fake_light_server(
    nodes: Value,
    fake: Fake,
) -> (SocketAddr, Arc<Mutex<Vec<Value>>>, Arc<Mutex<usize>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(Mutex::new(0));
    let nodes = Arc::new(Mutex::new(nodes));
    let (seen, count) = (commands.clone(), connections.clone());
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let first = {
                let mut count = count.lock().unwrap();
                *count += 1;
                *count == 1
            };
            let info = json!({ "fabric_id": 1, "schema_version": 13 });
            ws.send(Message::text(info.to_string())).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let command: Value = serde_json::from_str(&text).unwrap();
                let id = command["message_id"].clone();
                let reply = match command["command"].as_str().unwrap() {
                    "get_nodes" => json!({ "message_id": id, "result": *nodes.lock().unwrap() }),
                    "read_attribute" => {
                        seen.lock().unwrap().push(command.clone());
                        let args = &command["args"];
                        let node_id = args["node_id"].as_u64().unwrap();
                        let path = args["attribute_path"][0].as_str().unwrap().to_owned();
                        let endpoint: u64 = path.split('/').next().unwrap().parse().unwrap();
                        let value = if fake.read_off.contains(&(node_id, endpoint)) {
                            json!(false)
                        } else {
                            nodes
                                .lock()
                                .unwrap()
                                .as_array()
                                .unwrap()
                                .iter()
                                .find(|n| n["node_id"] == node_id)
                                .map_or(Value::Null, |n| n["attributes"][&path].clone())
                        };
                        json!({ "message_id": id, "result": { path: value } })
                    }
                    "device_command" => {
                        seen.lock().unwrap().push(command["args"].clone());
                        if fake.drop_first && first {
                            break;
                        }
                        let args = &command["args"];
                        let target = (
                            args["node_id"].as_u64().unwrap(),
                            args["endpoint_id"].as_u64().unwrap(),
                        );
                        if fake.silent.contains(&target) {
                            continue;
                        }
                        if fake.failing.contains(&target) {
                            json!({ "message_id": id, "error_code": 0, "details": "failed" })
                        } else {
                            if args["cluster_id"] == 6
                                && !(fake.ignore_off && args["command_name"] == "Off")
                            {
                                let on = args["command_name"] == "On";
                                for node in nodes.lock().unwrap().as_array_mut().unwrap() {
                                    if node["node_id"] == target.0 {
                                        node["attributes"][format!("{}/6/0", target.1)] = json!(on);
                                    }
                                }
                            }
                            json!({ "message_id": id, "result": null })
                        }
                    }
                    other => panic!("unexpected command {other}"),
                };
                ws.send(Message::text(reply.to_string())).await.unwrap();
            }
        }
    });
    (addr, commands, connections)
}

/// A bridge with an unreachable and a reachable bulb, a Wi-Fi bulb, an
/// unavailable node and a plug, as matterjs-server serves them.
fn home_nodes() -> Value {
    json!([
        { "node_id": 1, "available": true, "attributes": {
            "0/40/1": "Aqara", "0/40/15": "hub", "0/40/3": "Aqara Hub M3",
            "2/29/0": [{ "0": 19, "1": 2 }, { "0": 268, "1": 4 }], "2/6/0": false,
            "2/57/1": "Aqara", "2/57/15": "t2-a", "2/57/3": "Aqara LED Bulb T2", "2/57/17": false,
            "3/29/0": [{ "0": 19, "1": 2 }, { "0": 268, "1": 4 }], "3/6/0": false,
            "3/57/1": "Aqara", "3/57/15": "t2-b", "3/57/3": "Aqara LED Bulb T2", "3/57/17": true,
        }},
        { "node_id": 5, "available": true, "attributes": {
            "0/40/1": "Tapo", "0/40/15": TAPO, "0/40/3": "Smart Multicolor Bulb",
            "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": false,
        }},
        { "node_id": 16, "available": false, "attributes": {
            "0/40/1": "Uascent", "0/40/15": "beam", "0/40/3": "Smart Bulb",
            "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": true,
        }},
        { "node_id": 20, "available": true, "attributes": {
            "1/29/0": [{ "0": 266, "1": 1 }], "1/6/0": false,
        }},
    ])
}

async fn light_app(addr: SocketAddr) -> Router {
    let app = home_link::app(
        home_link::open_db(":memory:").unwrap(),
        Some(format!("ws://{addr}/ws")),
    );
    let room = create_room(&app, "寝室").await;
    for body in [
        json!({ "room_id": room, "vendor": "Tapo", "serial_number": TAPO, "name": "読書灯" }),
        json!({ "room_id": room, "vendor": "Aqara", "serial_number": "t2-b", "name": "台所" }),
        json!({ "room_id": room, "vendor": "Tapo", "serial_number": "gone", "name": "消えた灯" }),
    ] {
        assert_eq!(
            call(&app, "POST", "/api/devices", Some(body)).await.0,
            StatusCode::CREATED
        );
    }
    app
}

fn results(body: &Value, field: &str) -> Vec<(Value, Value, Value, Value)> {
    body["lights"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["node_id"].clone(),
                l["endpoint"].clone(),
                l["name"].clone(),
                l[field].clone(),
            )
        })
        .collect()
}

#[tokio::test]
async fn all_lights_switch_on_and_off_and_report_each_result() {
    let (addr, commands, _) = fake_light_server(home_nodes(), Fake::default()).await;
    let app = light_app(addr).await;

    let (status, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            &body["action"],
            &body["switched"],
            &body["no_response"],
            &body["failed"],
            &body["missing"]
        ),
        (&json!("on"), &json!(2), &json!(2), &json!(0), &json!(1))
    );
    assert_eq!(
        results(&body, "result"),
        [
            (
                json!(1),
                json!(2),
                json!("Aqara LED Bulb T2"),
                json!("no_response")
            ),
            (json!(1), json!(3), json!("台所"), json!("switched")),
            (json!(5), json!(1), json!("読書灯"), json!("switched")),
            (
                json!(16),
                json!(1),
                json!("Smart Bulb"),
                json!("no_response")
            ),
        ]
    );
    assert_eq!(
        body["missing_devices"],
        json!([{ "id": 3, "name": "消えた灯", "room_name": "寝室" }])
    );
    let sent: Vec<_> = commands
        .lock()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["node_id"].clone(),
                c["endpoint_id"].clone(),
                c["cluster_id"].clone(),
                c["command_name"].clone(),
            )
        })
        .collect();
    assert_eq!(
        sent,
        [
            (json!(1), json!(3), json!(6), json!("On")),
            (json!(5), json!(1), json!(6), json!("On"))
        ]
    );

    let (status, body) = call(&app, "GET", "/api/lights", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            &body["on"],
            &body["off"],
            &body["no_response"],
            &body["missing"]
        ),
        (&json!(2), &json!(0), &json!(2), &json!(1))
    );
    assert_eq!(
        results(&body, "state"),
        [
            (
                json!(1),
                json!(2),
                json!("Aqara LED Bulb T2"),
                json!("no_response")
            ),
            (json!(1), json!(3), json!("台所"), json!("on")),
            (json!(5), json!(1), json!("読書灯"), json!("on")),
            (
                json!(16),
                json!(1),
                json!("Smart Bulb"),
                json!("no_response")
            ),
        ]
    );

    let (_, body) = call(&app, "POST", "/api/lights/off", None).await;
    assert_eq!(
        (&body["action"], &body["switched"]),
        (&json!("off"), &json!(2))
    );
    let (_, body) = call(&app, "GET", "/api/lights", None).await;
    assert_eq!((&body["on"], &body["off"]), (&json!(0), &json!(2)));
}

#[tokio::test]
async fn lights_that_fail_or_never_answer_are_not_counted_as_switched() {
    let fake = Fake {
        failing: vec![(5, 1)],
        silent: vec![(1, 3)],
        ..Fake::default()
    };
    let (addr, _, _) = fake_light_server(home_nodes(), fake).await;
    let app = light_app(addr).await;

    let (status, body) = call(&app, "POST", "/api/lights/off", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (&body["switched"], &body["no_response"], &body["failed"]),
        (&json!(0), &json!(3), &json!(1))
    );
    assert_eq!(body["lights"][1]["result"], "no_response");
    assert_eq!(body["lights"][2]["result"], "failed");
}

#[tokio::test]
async fn switching_reconnects_when_the_connection_drops() {
    let fake = Fake {
        drop_first: true,
        ..Fake::default()
    };
    let (addr, commands, connections) = fake_light_server(home_nodes(), fake).await;
    let app = light_app(addr).await;

    let (status, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (&body["switched"], &body["no_response"]),
        (&json!(2), &json!(2))
    );
    assert_eq!(*connections.lock().unwrap(), 2);
    // On is absolute, so the light commanded before the drop is commanded again.
    assert_eq!(commands.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn lights_report_an_unset_or_unreachable_matter_server() {
    for (method, uri) in [
        ("GET", "/api/lights"),
        ("POST", "/api/lights/on"),
        ("POST", "/api/lights/off"),
    ] {
        let (status, error) = call(&app(), method, uri, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error["error"], "matter_server_not_configured");
    }
    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let app = home_link::app(
        home_link::open_db(":memory:").unwrap(),
        Some(format!("ws://{closed}/ws")),
    );
    for (method, uri) in [("GET", "/api/lights"), ("POST", "/api/lights/on")] {
        let (status, error) = call(&app, method, uri, None).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(error["error"], "matter_server_unreachable");
    }
}

/// A T2 behind a bridge that is on, another unreachable, a Tapo that is on, one
/// that is off, an unavailable BEAMTEC, and a plain On/Off light, with the
/// level and colour temperature ranges matterjs-server serves.
fn tuning_nodes() -> Value {
    json!([
        { "node_id": 1, "available": true, "attributes": {
            "2/29/0": [{ "0": 268, "1": 4 }], "2/6/0": false, "2/8/0": 100,
            "2/768/65532": 16, "2/57/3": "Aqara LED Bulb T2", "2/57/17": false,
            "3/29/0": [{ "0": 268, "1": 4 }], "3/6/0": true, "3/8/0": 100, "3/8/2": 1, "3/8/3": 254,
            "3/768/65532": 16, "3/768/16395": 153, "3/768/16396": 370,
            "3/57/1": "Aqara", "3/57/15": "t2-b", "3/57/3": "Aqara LED Bulb T2", "3/57/17": true,
        }},
        { "node_id": 5, "available": true, "attributes": {
            "0/40/1": "Tapo", "0/40/15": TAPO, "0/40/3": "Smart Multicolor Bulb",
            "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": true, "1/8/0": 100, "1/8/2": 0,
            "1/768/65532": 25, "1/768/16395": 153, "1/768/16396": 400,
        }},
        { "node_id": 6, "available": true, "attributes": {
            "0/40/3": "Smart Multicolor Bulb",
            "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": false, "1/8/0": 100,
            "1/768/65532": 25, "1/768/16395": 153, "1/768/16396": 400,
        }},
        { "node_id": 16, "available": false, "attributes": {
            "0/40/3": "Smart Bulb", "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": true,
            "1/8/0": 100, "1/768/65532": 16, "1/768/16395": 142, "1/768/16396": 454,
        }},
        { "node_id": 30, "available": true, "attributes": {
            "1/29/0": [{ "0": 256, "1": 1 }], "1/6/0": true,
        }},
    ])
}

/// 2026-10-05 23:00 in Tokyo: night, level 102 and 3000 K (333 mired).
const NIGHT: i64 = 1_791_208_800;

/// `(node_id, endpoint, cluster_id, command_name, payload)` of every command but On/Off.
fn tunings(commands: &Arc<Mutex<Vec<Value>>>) -> Vec<(Value, Value, Value, Value, Value)> {
    commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c["cluster_id"].is_u64() && c["cluster_id"] != 6)
        .map(|c| {
            (
                c["node_id"].clone(),
                c["endpoint_id"].clone(),
                c["cluster_id"].clone(),
                c["command_name"].clone(),
                c["payload"].clone(),
            )
        })
        .collect()
}

fn reads(commands: &Arc<Mutex<Vec<Value>>>) -> Vec<(Value, Value)> {
    commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c["command"] == "read_attribute")
        .map(|c| {
            (
                c["args"]["node_id"].clone(),
                c["args"]["attribute_path"].clone(),
            )
        })
        .collect()
}

fn decisions(run: &Value) -> Vec<(Value, Value, Value)> {
    run["lights"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["node_id"].clone(),
                l["endpoint"].clone(),
                l["decision"].clone(),
            )
        })
        .collect()
}

async fn enable(app: &Router) {
    let (status, body) = call(
        app,
        "PUT",
        "/api/lights/schedule",
        Some(json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

fn level(level: u64) -> Value {
    json!({ "level": level, "transitionTime": 300, "optionsMask": 0, "optionsOverride": 0 })
}

fn mireds(mireds: u64) -> Value {
    json!({ "colorTemperatureMireds": mireds, "transitionTime": 300, "optionsMask": 0, "optionsOverride": 0 })
}

#[tokio::test]
async fn scheduled_adjustment_writes_only_to_lights_read_as_on() {
    let fake = Fake {
        // Served as on, but the bulb itself answers off: it was switched off since.
        read_off: vec![(5, 1)],
        ..Fake::default()
    };
    let (addr, commands, _) = fake_light_server(tuning_nodes(), fake).await;
    let home = home_link::Home::new(
        home_link::open_db(":memory:").unwrap(),
        Some(format!("ws://{addr}/ws")),
    );
    let app = home.router();
    enable(&app).await;

    home.adjust(NIGHT, "scheduled").await;

    // The On/Off of every reachable light that can dim or change colour is read again.
    assert_eq!(
        reads(&commands),
        [
            (json!(1), json!(["3/6/0"])),
            (json!(5), json!(["1/6/0"])),
            (json!(6), json!(["1/6/0"])),
        ]
    );
    // Only the T2 read as on gets the night's values, without ExecuteIfOff.
    assert_eq!(
        tunings(&commands),
        [
            (
                json!(1),
                json!(3),
                json!(8),
                json!("MoveToLevel"),
                level(102)
            ),
            (
                json!(1),
                json!(3),
                json!(768),
                json!("MoveToColorTemperature"),
                mireds(333)
            ),
        ]
    );
    assert!(
        commands
            .lock()
            .unwrap()
            .iter()
            .all(|c| c["cluster_id"] != 6)
    );

    let (status, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let run = &body["runs"][0];
    assert_eq!(run["at"], "2026-10-05T23:00:00+09:00");
    assert_eq!(run["trigger"], "scheduled");
    assert_eq!((&run["level"], &run["kelvin"]), (&json!(102), &json!(3000)));
    assert_eq!(run["commands"], 2);
    assert_eq!(
        decisions(run),
        [
            (json!(1), json!(2), json!("no_response")),
            (json!(1), json!(3), json!("sent")),
            (json!(5), json!(1), json!("off")),
            (json!(6), json!(1), json!("off")),
            (json!(16), json!(1), json!("no_response")),
            (json!(30), json!(1), json!("unsupported")),
        ]
    );
    assert_eq!(run["lights"][1]["name"], "Aqara LED Bulb T2");
    assert_eq!(
        (&run["lights"][1]["level"], &run["lights"][1]["mireds"]),
        (&json!(102), &json!(333))
    );

    // Ten minutes later the night's values are unchanged: the T2 is neither read
    // nor written; the lights read as off are read again in case they came on.
    commands.lock().unwrap().clear();
    home.adjust(NIGHT + 600, "scheduled").await;
    assert_eq!(tunings(&commands), []);
    assert_eq!(
        reads(&commands),
        [(json!(5), json!(["1/6/0"])), (json!(6), json!(["1/6/0"]))]
    );
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["runs"][0]["lights"][1]["decision"], "unchanged");
    assert_eq!(body["runs"][0]["commands"], 0);
}

#[tokio::test]
async fn a_ledger_floor_keeps_a_light_at_or_above_its_colour_temperature() {
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let home = home_link::Home::new(
        home_link::open_db(":memory:").unwrap(),
        Some(format!("ws://{addr}/ws")),
    );
    let app = home.router();
    enable(&app).await;
    let room = create_room(&app, "リビング").await;
    let (_, tapo) = call(
        &app,
        "POST",
        "/api/devices",
        Some(json!({ "room_id": room, "vendor": "Tapo", "serial_number": TAPO, "name": "寝室前" })),
    )
    .await;
    let uri = format!("/api/devices/{}", tapo["id"]);
    for bad in [
        json!({ "min_kelvin": 999 }),
        json!({ "min_kelvin": 10001 }),
        json!({ "min": 4000 }),
    ] {
        let (status, _) = call(&app, "PATCH", &uri, Some(bad.clone())).await;
        assert!(status.is_client_error(), "{bad}");
    }
    let (status, device) = call(&app, "PATCH", &uri, Some(json!({ "min_kelvin": 4000 }))).await;
    assert_eq!(status, StatusCode::OK, "{device}");
    assert_eq!(device["min_kelvin"], 4000);
    let (status, _) = call(
        &app,
        "PATCH",
        "/api/devices/999",
        Some(json!({ "min_kelvin": 4000 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    home.adjust(NIGHT, "scheduled").await;

    // The Tapo gets 4000 K (250 mired) at night, the T2 without a floor 3000 K.
    let mireds_sent: Vec<_> = tunings(&commands)
        .into_iter()
        .filter(|(_, _, cluster, _, _)| cluster == 768)
        .map(|(node, _, _, _, payload)| (node, payload["colorTemperatureMireds"].clone()))
        .collect();
    assert_eq!(
        mireds_sent,
        [(json!(1), json!(333)), (json!(5), json!(250))]
    );
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(
        body["floors"],
        json!([{ "id": tapo["id"], "name": "寝室前", "room_name": "リビング", "min_kelvin": 4000 }])
    );

    // Removing the floor sends the Tapo 3000 K again.
    let (_, device) = call(&app, "PATCH", &uri, Some(json!({ "min_kelvin": null }))).await;
    assert_eq!(device["min_kelvin"], Value::Null);
    commands.lock().unwrap().clear();
    home.adjust(NIGHT + 600, "scheduled").await;
    assert_eq!(
        tunings(&commands),
        [(
            json!(5),
            json!(1),
            json!(768),
            json!("MoveToColorTemperature"),
            mireds(333)
        )]
    );
}

#[tokio::test]
async fn adjustment_is_off_until_the_user_enables_it() {
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let home = home_link::Home::new(
        home_link::open_db(":memory:").unwrap(),
        Some(format!("ws://{addr}/ws")),
    );
    home.adjust(NIGHT, "scheduled").await;
    let (_, body) = call(&home.router(), "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["settings"]["enabled"], false);
    assert_eq!(body["runs"][0]["commands"], 0);
    assert!(
        decisions(&body["runs"][0])
            .iter()
            .all(|(_, _, d)| d == "disabled")
    );
    assert!(commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn all_off_stops_adjustment_until_all_on_across_restarts() {
    let fake = Fake {
        ignore_off: true,
        ..Fake::default()
    };
    let (addr, commands, _) = fake_light_server(tuning_nodes(), fake).await;
    let url = Some(format!("ws://{addr}/ws"));
    let path = std::env::temp_dir().join(format!("home-link-intent-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let db_path = path.to_str().unwrap();
    {
        let home = home_link::Home::new(home_link::open_db(db_path).unwrap(), url.clone());
        let app = home.router();
        enable(&app).await;
        let (status, _) = call(&app, "POST", "/api/lights/off", None).await;
        assert_eq!(status, StatusCode::OK);
        home.adjust(NIGHT, "scheduled").await;
    }
    // Lights on again by other means (1/3 and 5/1) are left alone too.
    assert_eq!(tunings(&commands), []);
    assert_eq!(reads(&commands), []);

    // A restart keeps "all off".
    let home = home_link::Home::new(home_link::open_db(db_path).unwrap(), url);
    let app = home.router();
    // Every ten-minute run through the night and the next day sends nothing.
    for step in 1..=24 * 6 {
        home.adjust(NIGHT + 600 * step, "scheduled").await;
    }
    assert_eq!(tunings(&commands), []);
    assert_eq!(reads(&commands), []);
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["intent"]["action"], "off");
    assert_eq!(body["settings"]["enabled"], true);
    let run = &body["runs"][0];
    assert_eq!(run["at"], "2026-10-06T23:00:00+09:00");
    assert_eq!(run["commands"], 0);
    assert!(decisions(run).iter().all(|(_, _, d)| d == "all_off"));

    // "All on" lights them at the current values and resumes the adjustment.
    commands.lock().unwrap().clear();
    let (status, _) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["intent"]["action"], "on");
    assert_eq!(body["runs"][0]["trigger"], "lights_on");
    let tuned: Vec<_> = tunings(&commands)
        .into_iter()
        .map(|(node, endpoint, cluster, _, _)| (node, endpoint, cluster))
        .collect();
    assert_eq!(
        tuned,
        [
            (json!(1), json!(3), json!(8)),
            (json!(1), json!(3), json!(768)),
            (json!(5), json!(1), json!(8)),
            (json!(5), json!(1), json!(768)),
            (json!(6), json!(1), json!(8)),
            (json!(6), json!(1), json!(768)),
        ]
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn schedule_settings_merge_and_are_validated() {
    let app = app();
    let (status, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["settings"],
        json!({ "enabled": false, "latitude": 35.6895, "longitude": 139.6917,
                "morning_end_minute": 600, "day_level": 203, "night_level": 102,
                "warm_kelvin": 3000, "cool_kelvin": 5000 })
    );
    assert_eq!(body["intent"], Value::Null);
    assert_eq!(body["runs"], json!([]));

    let (status, body) = call(
        &app,
        "PUT",
        "/api/lights/schedule",
        Some(json!({ "night_level": 20, "latitude": 34.69, "longitude": 135.50 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["settings"]["night_level"], 20);
    assert_eq!(body["settings"]["day_level"], 203);

    for bad in [
        json!({ "night_level": 0 }),
        json!({ "warm_kelvin": 6000 }),
        json!({ "latitude": 91 }),
        json!({ "morning_end_minute": 1320 }),
        json!({ "enable": true }),
        json!({ "enabled": "yes" }),
        json!([]),
    ] {
        let (status, error) = call(&app, "PUT", "/api/lights/schedule", Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(error["error"], "invalid_schedule", "{bad}");
    }
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["settings"]["night_level"], 20);
}

/// How the fake matterjs-server answers `commission_with_code`.
#[derive(Clone)]
enum Commission {
    /// The node as matterjs-server returns it.
    Node(Value),
    /// An error with these details, as matterjs-server words a failed commissioning.
    Error(&'static str),
    /// No answer at all.
    Silent,
}

/// Serves server info (with `bluetooth_enabled`), `set_wifi_credentials`,
/// `commission_with_code` and `read_attribute` the way matterjs-server does,
/// recording every command.
async fn fake_commissioner(
    bluetooth: bool,
    commission: Commission,
    read: Value,
) -> (SocketAddr, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let seen = commands.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let info =
                json!({ "fabric_id": 1, "schema_version": 13, "bluetooth_enabled": bluetooth });
            ws.send(Message::text(info.to_string())).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let command: Value = serde_json::from_str(&text).unwrap();
                seen.lock().unwrap().push(command.clone());
                let id = command["message_id"].clone();
                let reply = match (command["command"].as_str().unwrap(), &commission) {
                    ("set_wifi_credentials", _) => json!({ "message_id": id, "result": {} }),
                    ("commission_with_code", Commission::Node(node)) => {
                        let event = json!({ "event": "node_added", "data": node });
                        ws.send(Message::text(event.to_string())).await.unwrap();
                        json!({ "message_id": id, "result": node })
                    }
                    ("commission_with_code", Commission::Error(details)) => {
                        json!({ "message_id": id, "error_code": 1, "details": details })
                    }
                    ("commission_with_code", Commission::Silent) => continue,
                    ("read_attribute", _) => json!({ "message_id": id, "result": read }),
                    (other, _) => panic!("unexpected command {other}"),
                };
                ws.send(Message::text(reply.to_string())).await.unwrap();
            }
        }
    });
    (addr, commands)
}

const PASSWORD: &str = "kakushi-pass-7f3a";

fn commission_body(room: i64) -> Value {
    json!({ "room_id": room, "qr_payload": QR, "name": "押入れ1",
            "wifi_ssid": "home-2g", "wifi_password": PASSWORD })
}

/// A Tapo bulb as matterjs-server returns it right after commissioning.
fn tapo_node() -> Value {
    json!({ "node_id": 17, "available": true, "attributes": {
        "0/40/1": "Tapo", "0/40/15": TAPO, "0/40/3": "Smart Bulb",
        "0/51/0": [{ "0": "wlan0", "4": "zLq94MJE", "7": 1 }],
    }})
}

fn commissioning_app(addr: SocketAddr, db: rusqlite::Connection) -> Router {
    home_link::Home::new(db, Some(format!("ws://{addr}/ws")))
        .with_commission_timeout(std::time::Duration::from_millis(500))
        .router()
}

#[tokio::test]
async fn commissioning_hands_over_wifi_then_registers_the_new_node() {
    let (addr, commands) = fake_commissioner(true, Commission::Node(tapo_node()), json!({})).await;
    let path = std::env::temp_dir().join(format!("home-link-commission-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let app = commissioning_app(addr, home_link::open_db(path.to_str().unwrap()).unwrap());
    let room = create_room(&app, "押入れ").await;

    let (status, body) = call(&app, "POST", "/api/commission", Some(commission_body(room))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["node_id"], 17);
    assert_eq!(body["registered"], true);
    let device = &body["device"];
    assert_eq!(
        (
            &device["room_name"],
            &device["name"],
            &device["vendor"],
            &device["serial_number"],
            &device["mac"]
        ),
        (
            &json!("押入れ"),
            &json!("押入れ1"),
            &json!("Tapo"),
            &json!(TAPO),
            &json!(TAPO)
        )
    );
    let commands = commands.lock().unwrap().clone();
    assert_eq!(
        commands
            .iter()
            .map(|c| c["command"].clone())
            .collect::<Vec<_>>(),
        [json!("set_wifi_credentials"), json!("commission_with_code")]
    );
    assert_eq!(
        commands[0]["args"],
        json!({ "ssid": "home-2g", "credentials": PASSWORD })
    );
    assert_eq!(
        commands[1]["args"],
        json!({ "code": QR, "network_only": false })
    );

    // The ledger keeps the code but never the Wi-Fi password.
    let id = device["id"].as_i64().unwrap();
    let (_, stored) = call(&app, "GET", &format!("/api/devices/{id}"), None).await;
    assert_eq!(stored["qr_payload"], QR);
    assert!(!body.to_string().contains(PASSWORD));
    let (_, all) = call(&app, "GET", "/api/devices", None).await;
    assert!(!all.to_string().contains(PASSWORD));
    drop(app);
    for suffix in ["", "-wal", "-shm"] {
        let file = format!("{}{suffix}", path.display());
        if let Ok(bytes) = std::fs::read(&file) {
            assert!(
                !bytes
                    .windows(PASSWORD.len())
                    .any(|w| w == PASSWORD.as_bytes()),
                "{file} holds the password"
            );
        }
        let _ = std::fs::remove_file(&file);
    }
}

#[tokio::test]
async fn commissioning_reads_identifiers_missing_from_the_new_node() {
    let node = json!({ "node_id": 18, "attributes": {} });
    let read = json!({ "0/40/1": "Uascent", "0/40/15": "U2025", "0/51/0": [] });
    let (addr, commands) = fake_commissioner(true, Commission::Node(node), read).await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let room = create_room(&app, "押入れ").await;
    let body = json!({ "room_id": room, "manual_code": "3497-011-2332",
                       "wifi_ssid": "home-2g", "wifi_password": PASSWORD });
    let (status, body) = call(&app, "POST", "/api/commission", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["device"]["vendor"], "Uascent");
    assert_eq!(body["device"]["serial_number"], "U2025");
    assert_eq!(body["device"]["mac"], Value::Null);
    assert_eq!(body["device"]["name"], "");
    let commands = commands.lock().unwrap().clone();
    assert_eq!(commands[1]["args"]["code"], MANUAL);
    assert_eq!(
        commands[2]["args"],
        json!({ "node_id": 18, "attribute_path": ["0/40/1", "0/40/15", "0/51/0"] })
    );
}

#[tokio::test]
async fn commissioning_failures_are_told_apart_and_register_nothing() {
    let cases = [
        (
            Commission::Error(
                "Commission failed: commissioning discovery failed: No commissionable device was discovered",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "device_not_found",
        ),
        (
            Commission::Error(
                "Commission failed: commissioning discovery failed: No device could be commissioned (1 of 1 started attempt(s) failed, 1 discovered)",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong_code",
        ),
        (
            Commission::Error(
                "Commission failed: Commissionee failed to connect to WiFi network \"home-2g\": AuthFail",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wifi_failed",
        ),
        (
            Commission::Error(
                "Commission failed: Commissioning time exceeds the maximum timeframe of 300s",
            ),
            StatusCode::GATEWAY_TIMEOUT,
            "commission_timeout",
        ),
        (
            Commission::Silent,
            StatusCode::GATEWAY_TIMEOUT,
            "commission_timeout",
        ),
        (
            Commission::Error(
                "Commission failed: Commission error: This device is already commissioned into this fabric.",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "commission_failed",
        ),
    ];
    for (commission, status, code) in cases {
        let (addr, _) = fake_commissioner(true, commission, json!({})).await;
        let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
        let room = create_room(&app, "押入れ").await;
        let (got, error) = call(&app, "POST", "/api/commission", Some(commission_body(room))).await;
        assert_eq!(
            (got, error["error"].as_str().unwrap()),
            (status, code),
            "{error}"
        );
        assert!(!error.to_string().contains(PASSWORD));
        let (_, devices) = call(&app, "GET", "/api/devices", None).await;
        assert_eq!(devices, json!([]), "{code}");
    }
}

#[tokio::test]
async fn commissioning_a_device_already_in_the_ledger_keeps_one_entry() {
    let (addr, _) = fake_commissioner(true, Commission::Node(tapo_node()), json!({})).await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let room = create_room(&app, "寝室").await;
    // Recorded by its code before, without identifiers.
    let (_, old) = call(
        &app,
        "POST",
        "/api/devices",
        Some(json!({ "room_id": room, "manual_code": MANUAL, "name": "読書灯" })),
    )
    .await;
    let closet = create_room(&app, "押入れ").await;

    let (status, body) = call(
        &app,
        "POST",
        "/api/commission",
        Some(commission_body(closet)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["registered"], false);
    assert_eq!(body["device"]["id"], old["id"]);
    assert_eq!(body["device"]["room_name"], "寝室");
    assert_eq!(body["device"]["name"], "読書灯");
    // The entry gains the identifiers it lacked.
    assert_eq!(body["device"]["serial_number"], TAPO);
    assert_eq!(body["device"]["mac"], TAPO);
    let (_, devices) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(devices.as_array().unwrap().len(), 1);

    // Again, now found by its identifiers.
    let (status, body) = call(
        &app,
        "POST",
        "/api/commission",
        Some(commission_body(closet)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["device"]["id"], old["id"]);
}

#[tokio::test]
async fn commissioning_checks_its_input_before_reaching_the_matter_server() {
    let (addr, commands) = fake_commissioner(true, Commission::Node(tapo_node()), json!({})).await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let room = create_room(&app, "押入れ").await;
    let with = |field: &str, value: Value| {
        let mut body = commission_body(room);
        body[field] = value;
        body
    };
    let cases = [
        (
            with("qr_payload", json!("https://example.com")),
            StatusCode::BAD_REQUEST,
            "invalid_qr_payload",
        ),
        (
            with("qr_payload", Value::Null),
            StatusCode::BAD_REQUEST,
            "missing_setup_code",
        ),
        (
            with("manual_code", json!(MANUAL)),
            StatusCode::BAD_REQUEST,
            "missing_setup_code",
        ),
        (
            with("wifi_ssid", json!("")),
            StatusCode::BAD_REQUEST,
            "invalid_wifi",
        ),
        (
            with("wifi_ssid", json!("x".repeat(33))),
            StatusCode::BAD_REQUEST,
            "invalid_wifi",
        ),
        (
            with("wifi_password", json!("")),
            StatusCode::BAD_REQUEST,
            "invalid_wifi",
        ),
        (
            with("wifi_password", json!("x".repeat(65))),
            StatusCode::BAD_REQUEST,
            "invalid_wifi",
        ),
        (
            with("name", json!("x".repeat(101))),
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
        ),
        (
            with("room_id", json!(999)),
            StatusCode::NOT_FOUND,
            "room_not_found",
        ),
    ];
    for (body, status, code) in cases {
        let (got, error) = call(&app, "POST", "/api/commission", Some(body)).await;
        assert_eq!(
            (got, error["error"].as_str().unwrap()),
            (status, code),
            "{error}"
        );
    }
    assert_eq!(commands.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn commissioning_needs_a_matter_server_with_bluetooth() {
    let (addr, commands) = fake_commissioner(false, Commission::Node(tapo_node()), json!({})).await;
    let ble_off = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let room = create_room(&ble_off, "押入れ").await;
    let (status, error) = call(
        &ble_off,
        "POST",
        "/api/commission",
        Some(commission_body(room)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"], "bluetooth_unavailable");
    assert_eq!(commands.lock().unwrap().len(), 0);

    let unset = app();
    let room = create_room(&unset, "押入れ").await;
    let (status, error) = call(
        &unset,
        "POST",
        "/api/commission",
        Some(commission_body(room)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"], "matter_server_not_configured");

    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let app = commissioning_app(closed, home_link::open_db(":memory:").unwrap());
    let room = create_room(&app, "押入れ").await;
    let (status, error) = call(&app, "POST", "/api/commission", Some(commission_body(room))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(error["error"], "matter_server_unreachable");
}
