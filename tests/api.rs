use std::net::SocketAddr;

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
        let row: (String, String, Option<String>) = db
            .query_row(
                "SELECT qr_payload, name, serial_number FROM devices WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("34970112332".into(), "電球".into(), None));
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
