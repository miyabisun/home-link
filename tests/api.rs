#![cfg(test)]

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
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

/// Two connections to a fresh in-memory ledger: one for the app, one for the
/// test to record devices in, as only commissioning does through the API.
fn ledger() -> (rusqlite::Connection, rusqlite::Connection) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let uri = format!("file:ledger-{n}?mode=memory&cache=shared");
    let open = || home_link::open_db(&uri).unwrap();
    (open(), open())
}

/// An app on a fresh `ledger()`, with the test's connection to it.
fn ledger_app() -> (Router, rusqlite::Connection) {
    let (db, sql) = ledger();
    (home_link::app(db, None), sql)
}

/// Records `device` (a code and identifiers as stored, `name`) and
/// returns it as `GET /api/devices/{id}` shows it.
async fn add_device(app: &Router, db: &rusqlite::Connection, device: Value) -> Value {
    let text = |key| device.get(key).and_then(Value::as_str);
    db.execute(
        "INSERT INTO devices (qr_payload, vendor, serial_number, mac, name)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            text("qr_payload").or(text("manual_code")),
            text("vendor"),
            text("serial_number"),
            text("mac"),
            text("name").unwrap_or(""),
        ],
    )
    .unwrap();
    let uri = format!("/api/devices/{}", db.last_insert_rowid());
    let (status, device) = call(app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{device}");
    device
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

const TAPO: &str = "AABBCCDDEEFF";

#[tokio::test]
async fn devices_hide_their_codes_in_list_but_not_alone() {
    let (app, sql) = ledger_app();
    let qr = add_device(&app, &sql, json!({ "qr_payload": QR })).await;
    let manual = json!({ "manual_code": MANUAL, "name": "電球" });
    let manual = add_device(&app, &sql, manual).await;
    let tapo = json!({ "vendor": "Tapo", "serial_number": TAPO });
    let tapo = add_device(&app, &sql, tapo).await;

    let (status, devices) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(devices.as_array().unwrap().len(), 3);
    assert_eq!(devices[1]["name"], "電球");
    assert_eq!(devices[2]["serial_number"], TAPO);
    assert!(!devices.to_string().contains("Y.K9042C00"));
    assert!(!devices.to_string().contains("3497"));

    assert_eq!(qr["qr_payload"], QR);
    assert!(qr.get("manual_code").is_none());
    assert_eq!(manual["manual_code"], MANUAL);
    assert!(manual.get("qr_payload").is_none());
    assert!(tapo.get("qr_payload").is_none() && tapo.get("manual_code").is_none());

    let uri = format!("/api/devices/{}", manual["id"]);
    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, error) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"], "device_not_found");
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
        let row: (String, String, Option<String>, i64) = db
            .query_row(
                "SELECT qr_payload, name, serial_number,
                    (SELECT count(*) FROM device_labels WHERE device_id = devices.id)
                 FROM devices WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        // Adjusted by the schedule before, it is under the auto label.
        assert_eq!(row, ("34970112332".into(), "電球".into(), None, 1));
        db.execute(
            "INSERT INTO devices (vendor, serial_number) VALUES ('Tapo', ?1)",
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
    let (db, sql) = ledger();
    let app = home_link::app(db, Some(format!("ws://{addr}/ws")));
    for body in [
        json!({ "vendor": "Tapo", "serial_number": TAPO, "name": "A" }),
        json!({ "vendor": "Aqara", "serial_number": "54ef441001372c4c", "name": "B" }),
        json!({ "vendor": "Tapo", "serial_number": "AABBCCDD0000", "name": "C" }),
        json!({ "manual_code": MANUAL, "name": "D" }),
    ] {
        add_device(&app, &sql, body).await;
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
                d["visible"].clone(),
                d["node_id"].clone(),
                d["endpoint"].clone(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (json!("A"), json!(true), json!(5), json!(0)),
            (json!("B"), json!(true), json!(1), json!(3)),
            (json!("C"), json!(false), Value::Null, Value::Null),
            (json!("D"), json!(false), Value::Null, Value::Null),
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
    /// Close the connection when the first command to a light arrives.
    drop_first: bool,
    /// `(node_id, endpoint)` read as off although `get_nodes` serves it on.
    read_off: Vec<(u64, u64)>,
    /// Off is accepted but the lights stay on, as if switched on again by other means.
    ignore_off: bool,
    /// `(node_id, endpoint)` whose On is accepted before the bridge reports it:
    /// the served and read On/Off stay off.
    late_on: Vec<(u64, u64)>,
    /// `(node_id, endpoint)` that leaves its first command unanswered.
    silent_first: Vec<(u64, u64)>,
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
    let answered = Arc::new(Mutex::new(Vec::new()));
    let dropped = std::sync::atomic::AtomicBool::new(false);
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            *count.lock().unwrap() += 1;
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
                        if fake.drop_first && !dropped.swap(true, Ordering::Relaxed) {
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
                        if fake.silent_first.contains(&target) {
                            let mut answered = answered.lock().unwrap();
                            if !answered.contains(&target) {
                                answered.push(target);
                                continue;
                            }
                        }
                        if fake.failing.contains(&target) {
                            json!({ "message_id": id, "error_code": 0, "details": "failed" })
                        } else {
                            let lights = args["command_name"] == "MoveToLevelWithOnOff";
                            if (args["cluster_id"] == 6 || lights)
                                && !(fake.ignore_off && args["command_name"] == "Off")
                                && !(fake.late_on.contains(&target)
                                    && args["command_name"] != "Off")
                            {
                                let on = args["command_name"] == "On" || lights;
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

async fn light_app(addr: SocketAddr) -> (Router, rusqlite::Connection) {
    let (db, sql) = ledger();
    let app = home_link::app(db, Some(format!("ws://{addr}/ws")));
    for body in [
        json!({ "vendor": "Tapo", "serial_number": TAPO, "name": "読書灯" }),
        json!({ "vendor": "Aqara", "serial_number": "t2-b", "name": "台所" }),
        json!({ "vendor": "Tapo", "serial_number": "gone", "name": "消えた灯" }),
    ] {
        add_device(&app, &sql, body).await;
    }
    (app, sql)
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
    let (app, _) = light_app(addr).await;

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
        json!([{ "id": 3, "name": "消えた灯" }])
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
    let (app, _) = light_app(addr).await;

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
    let (app, _) = light_app(addr).await;
    // Recording the ledger reads the nodes too.
    let before = *connections.lock().unwrap();

    let (status, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (&body["switched"], &body["no_response"]),
        (&json!(2), &json!(2))
    );
    assert_eq!(*connections.lock().unwrap() - before, 2);
    // On is absolute, so the light commanded before the drop is commanded again.
    assert_eq!(commands.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn lights_that_miss_the_first_on_are_sent_it_again() {
    let fake = Fake {
        silent_first: vec![(5, 1)],
        ..Fake::default()
    };
    let (addr, commands, _) = fake_light_server(home_nodes(), fake).await;
    let (app, _) = light_app(addr).await;

    let (status, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (&body["switched"], &body["no_response"]),
        (&json!(2), &json!(2))
    );
    assert_eq!(body["lights"][2]["result"], "switched");
    let to_tapo = commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c["node_id"] == 5 && c["command_name"] == "On")
        .count();
    assert_eq!(to_tapo, 2);
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
            "2/768/65532": 16, "2/57/1": "Aqara", "2/57/15": "t2-a",
            "2/57/3": "Aqara LED Bulb T2", "2/57/17": false,
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
            "0/40/1": "Tapo", "0/40/15": "tapo-desk", "0/40/3": "Smart Multicolor Bulb",
            "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": false, "1/8/0": 100,
            "1/768/65532": 25, "1/768/16395": 153, "1/768/16396": 400,
        }},
        { "node_id": 16, "available": false, "attributes": {
            "0/40/1": "Uascent", "0/40/15": "beam",
            "0/40/3": "Smart Bulb", "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": true,
            "1/8/0": 100, "1/768/65532": 16, "1/768/16395": 142, "1/768/16396": 454,
        }},
        { "node_id": 30, "available": true, "attributes": {
            "0/40/1": "Plainco", "0/40/15": "plain",
            "1/29/0": [{ "0": 256, "1": 1 }], "1/6/0": true,
        }},
    ])
}

/// The vendor and serial number of every light `tuning_nodes` serves, in its order.
const TUNING_LIGHTS: [(&str, &str); 6] = [
    ("Aqara", "t2-a"),
    ("Aqara", "t2-b"),
    ("Tapo", TAPO),
    ("Tapo", "tapo-desk"),
    ("Uascent", "beam"),
    ("Plainco", "plain"),
];

/// A home over `tuning_nodes` whose lights are all in the ledger, unnamed, under the
/// auto label "ambient", with the test's connection to the ledger.
async fn auto_home(
    addr: SocketAddr,
    clock: Option<fn() -> i64>,
) -> (home_link::Home, Router, rusqlite::Connection) {
    let (db, sql) = ledger();
    let home = home_link::Home::new(db, Some(format!("ws://{addr}/ws")));
    let home = match clock {
        Some(clock) => home.with_clock(clock),
        None => home,
    };
    let app = home.router();
    let ambient = create_label(&app, json!({ "name": "ambient", "auto": true })).await;
    for (vendor, serial) in TUNING_LIGHTS {
        let device = add_device(
            &app,
            &sql,
            json!({ "vendor": vendor, "serial_number": serial }),
        )
        .await;
        assert_eq!(assign(&app, &device, &[&ambient]).await.0, StatusCode::OK);
    }
    (home, app, sql)
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
    let (home, app, _) = auto_home(addr, None).await;
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

fn sent_values(commands: &Arc<Mutex<Vec<Value>>>) -> Vec<(Value, Value, Value)> {
    tunings(commands)
        .into_iter()
        .map(|(node, endpoint, _, _, payload)| {
            let value = if payload["level"].is_u64() {
                payload["level"].clone()
            } else {
                payload["colorTemperatureMireds"].clone()
            };
            (node, endpoint, value)
        })
        .collect()
}

#[tokio::test]
async fn labels_support_create_list_update_assign_and_delete() {
    let (app, sql) = ledger_app();
    let device = add_device(
        &app,
        &sql,
        json!({ "vendor": "Tapo", "serial_number": TAPO, "name": "ゲームPC" }),
    )
    .await;
    let device_uri = format!("/api/devices/{}", device["id"]);
    assert_eq!(device["labels"], json!([]));

    let (status, label) = call(
        &app,
        "POST",
        "/api/labels",
        Some(json!({ "name": " work ", "override_level": 254, "override_kelvin": 5000 })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{label}");
    let id = label["id"].clone();
    assert_eq!(
        label,
        json!({ "id": id, "name": "work", "auto": false, "override_level": 254,
                "override_kelvin": 5000, "on": null, "schedule": [], "device_count": 0,
                "boosted": false })
    );

    // Assigning and the device's view of it.
    let (status, assigned) = call(
        &app,
        "PATCH",
        &device_uri,
        Some(json!({ "label_ids": [id] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");
    assert_eq!(assigned["labels"], json!([{ "id": id, "name": "work" }]));
    let (_, labels) = call(&app, "GET", "/api/labels", None).await;
    assert_eq!(labels[0]["device_count"], 1);

    // Updating merges; null takes the override away; a schedule replaces the last.
    let uri = format!("/api/labels/{id}");
    let (status, label) = call(
        &app,
        "PATCH",
        &uri,
        Some(json!({ "override_level": null, "override_kelvin": null, "auto": true,
                     "schedule": [{ "time": "05:00", "on": false }, { "time": "00:00", "on": true }] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{label}");
    assert_eq!(
        (
            &label["name"],
            &label["auto"],
            &label["override_level"],
            &label["override_kelvin"]
        ),
        (&json!("work"), &json!(true), &Value::Null, &Value::Null)
    );
    assert_eq!(
        label["schedule"],
        json!([{ "time": "00:00", "on": true }, { "time": "05:00", "on": false }])
    );
    let (_, label) = call(&app, "PATCH", &uri, Some(json!({ "name": "ambient" }))).await;
    assert_eq!(
        (&label["auto"], label["schedule"].as_array().unwrap().len()),
        (&json!(true), 2)
    );
    let (_, label) = call(&app, "PATCH", &uri, Some(json!({ "schedule": [] }))).await;
    assert_eq!(label["schedule"], json!([]));

    // Deleting a label in use unassigns its devices.
    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, device) = call(&app, "GET", &device_uri, None).await;
    assert_eq!(device["labels"], json!([]));
    let (status, error) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(
        (status, &error["error"]),
        (StatusCode::NOT_FOUND, &json!("label_not_found"))
    );
    let (_, labels) = call(&app, "GET", "/api/labels", None).await;
    assert_eq!(labels, json!([]));

    // Unassigning with an empty list.
    let (_, label) = call(&app, "POST", "/api/labels", Some(json!({ "name": "夜" }))).await;
    call(
        &app,
        "PATCH",
        &device_uri,
        Some(json!({ "label_ids": [label["id"]] })),
    )
    .await;
    let (status, device) = call(&app, "PATCH", &device_uri, Some(json!({ "label_ids": [] }))).await;
    assert_eq!((status, &device["labels"]), (StatusCode::OK, &json!([])));
}

#[tokio::test]
async fn label_traits_names_and_assignments_are_validated() {
    let (app, sql) = ledger_app();
    let device = add_device(
        &app,
        &sql,
        json!({ "vendor": "Tapo", "serial_number": TAPO }),
    )
    .await;
    let device_uri = format!("/api/devices/{}", device["id"]);
    let (_, label) = call(&app, "POST", "/api/labels", Some(json!({ "name": "game" }))).await;
    let uri = format!("/api/labels/{}", label["id"]);
    for bad in [
        json!({ "name": "x", "override_level": 0, "override_kelvin": 5000 }),
        json!({ "name": "x", "override_level": 255, "override_kelvin": 5000 }),
        json!({ "name": "x", "override_level": 254, "override_kelvin": 999 }),
        json!({ "name": "x", "override_level": 254, "override_kelvin": 10001 }),
        // Both or neither.
        json!({ "name": "x", "override_level": 254 }),
        json!({ "name": "x", "override_kelvin": 5000 }),
        // An override holds its lights out of the adjustment, so it is not also auto.
        json!({ "name": "x", "auto": true, "override_level": 254, "override_kelvin": 5000 }),
        json!({ "name": "x", "auto": "yes" }),
        json!({ "name": "x", "schedule": [{ "time": "24:00", "on": true }] }),
        json!({ "name": "x", "schedule": [{ "time": "5:00", "on": true }] }),
        json!({ "name": "x", "schedule": [{ "time": "05:60", "on": true }] }),
        json!({ "name": "x", "schedule": [{ "time": "05:00" }] }),
        json!({ "name": "x", "schedule": [{ "time": "05:00", "on": true }, { "time": "05:00", "on": false }] }),
        json!({ "name": "x", "schedule": { "time": "05:00", "on": true } }),
        // The values of each label and being dedicated are gone; the state is switched, not set.
        json!({ "name": "x", "night_level": 76 }),
        json!({ "name": "x", "dedicated": true }),
        json!({ "name": "x", "on": true }),
    ] {
        let (status, error) = call(&app, "POST", "/api/labels", Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(error["error"], "invalid_label", "{bad}");
    }
    for bad in [
        json!({}),
        json!({ "name": "  " }),
        json!({ "name": "あ".repeat(101) }),
    ] {
        let (status, error) = call(&app, "POST", "/api/labels", Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(error["error"], "invalid_label_name");
    }
    let (status, error) = call(&app, "POST", "/api/labels", Some(json!({ "name": "game" }))).await;
    assert_eq!(
        (status, &error["error"]),
        (StatusCode::CONFLICT, &json!("duplicate_label_name"))
    );
    let (status, _) = call(&app, "PATCH", &uri, Some(json!({ "override_level": 900 }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, error) = call(
        &app,
        "PATCH",
        &device_uri,
        Some(json!({ "label_ids": [999] })),
    )
    .await;
    assert_eq!(
        (status, &error["error"]),
        (StatusCode::NOT_FOUND, &json!("label_not_found"))
    );
    // An empty body does not remove the label.
    let (status, error) = call(&app, "PATCH", &device_uri, Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_device_update");
    for bad in [
        json!({ "min_kelvin": 3500 }),
        json!({ "label_id": label["id"] }),
        json!({ "label_ids": null }),
        json!({ "label_ids": label["id"] }),
        json!({ "label_ids": ["game"] }),
    ] {
        let (status, error) = call(&app, "PATCH", &device_uri, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(error["error"], "invalid_device_update", "{bad}");
    }
    let (status, _) = call(
        &app,
        "PATCH",
        "/api/labels/999",
        Some(json!({ "name": "y" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A label row: id, name, auto, override level and kelvin.
type LabelRow = (i64, String, bool, Option<u8>, Option<u16>);

/// Each device with the ids of its labels.
fn assignments(db: &rusqlite::Connection) -> Vec<(i64, Vec<i64>)> {
    db.prepare(
        "SELECT devices.id, json_group_array(label_id) FILTER (WHERE label_id IS NOT NULL)
         FROM devices LEFT JOIN device_labels ON device_id = devices.id
         GROUP BY devices.id ORDER BY devices.id",
    )
    .unwrap()
    .query_map([], |r| {
        let ids: String = r.get(1)?;
        let mut ids: Vec<i64> = serde_json::from_str(&ids).unwrap();
        ids.sort_unstable();
        Ok((r.get(0)?, ids))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn label_rows(db: &rusqlite::Connection) -> Vec<LabelRow> {
    db.prepare("SELECT id, name, auto, override_level, override_kelvin FROM labels ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn opening_a_ledger_with_floors_moves_them_to_labels() {
    let path = std::env::temp_dir().join(format!("home-link-labels-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        // A version 2 ledger: three Tapo with 3500 K, one with 4000 K and a T2 without.
        let old = home_link::open_db(path.to_str().unwrap()).unwrap();
        old.execute_batch(
            "PRAGMA user_version = 0;
            DROP TABLE label_schedules;
            DROP TABLE device_labels;
            DROP TABLE devices;
            DROP TABLE labels;
            CREATE TABLE rooms (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
            CREATE TABLE devices (
                id INTEGER PRIMARY KEY,
                room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
                qr_payload TEXT UNIQUE,
                vendor TEXT,
                serial_number TEXT,
                mac TEXT UNIQUE,
                name TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                min_kelvin INTEGER,
                UNIQUE (vendor, serial_number),
                CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
            );
            PRAGMA user_version = 2;
            INSERT INTO rooms (name) VALUES ('リビング');
            INSERT INTO devices (id, room_id, vendor, serial_number, min_kelvin) VALUES
                (3, 1, 'Tapo', 'a', 3500), (4, 1, 'Tapo', 'b', 3500), (14, 1, 'Tapo', 'c', 3500),
                (15, 1, 'Tapo', 'd', 4000), (20, 1, 'Aqara', 't2', NULL);",
        )
        .unwrap();
    }
    for _ in 0..2 {
        let db = home_link::open_db(path.to_str().unwrap()).unwrap();
        // Each label became one that lights at the day's values, and every light the
        // schedule adjusted is under the new auto label.
        assert_eq!(
            label_rows(&db),
            [
                (1, "夜3500K".into(), false, Some(203), Some(5000)),
                (2, "夜4000K".into(), false, Some(203), Some(5000)),
                (3, "自動調光".into(), true, None, None),
            ]
        );
        assert_eq!(
            assignments(&db),
            [
                (3, vec![1, 3]),
                (4, vec![1, 3]),
                (14, vec![1, 3]),
                (15, vec![2, 3]),
                (20, vec![3])
            ]
        );
        let has_floor: bool = db
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM pragma_table_info('devices') WHERE name = 'min_kelvin')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_floor);
    }
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn adjustment_is_off_until_the_user_enables_it() {
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let (home, app, _) = auto_home(addr, None).await;
    home.adjust(NIGHT, "scheduled").await;
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
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
#[allow(clippy::too_many_lines)]
async fn an_auto_labels_off_stops_its_adjustment_until_it_is_on_across_restarts() {
    let fake = Fake {
        ignore_off: true,
        ..Fake::default()
    };
    let (addr, commands, _) = fake_light_server(tuning_nodes(), fake).await;
    let url = Some(format!("ws://{addr}/ws"));
    let path = std::env::temp_dir().join(format!("home-link-intent-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let db_path = path.to_str().unwrap();
    let ambient = {
        let home = home_link::Home::new(home_link::open_db(db_path).unwrap(), url.clone());
        let app = home.router();
        let sql = home_link::open_db(db_path).unwrap();
        enable(&app).await;
        let ambient = create_label(&app, json!({ "name": "ambient", "auto": true })).await;
        for (vendor, serial) in TUNING_LIGHTS {
            let device = add_device(
                &app,
                &sql,
                json!({ "vendor": vendor, "serial_number": serial }),
            )
            .await;
            assert_eq!(assign(&app, &device, &[&ambient]).await.0, StatusCode::OK);
        }
        let (status, body) = call(
            &app,
            "POST",
            &format!("/api/labels/{}/off", ambient["id"]),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (&body["action"], &body["on"]),
            (&json!("off"), &json!(false))
        );
        commands.lock().unwrap().clear();
        home.adjust(NIGHT, "scheduled").await;
        ambient
    };
    // Lights on again by other means (1/3 and 5/1) are left alone too.
    assert_eq!(tunings(&commands), []);
    assert_eq!(reads(&commands), []);

    // A restart keeps the label off.
    let home = home_link::Home::new(home_link::open_db(db_path).unwrap(), url.clone());
    let app = home.router();
    // Every ten-minute run through the night and the next day sends nothing.
    for step in 1..=24 * 6 {
        home.adjust(NIGHT + 600 * step, "scheduled").await;
    }
    assert_eq!(tunings(&commands), []);
    assert_eq!(reads(&commands), []);
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert!(body.get("intent").is_none(), "{body}");
    assert_eq!(body["labels"][0]["on"], false);
    let run = &body["runs"][0];
    assert_eq!(run["at"], "2026-10-06T23:00:00+09:00");
    assert_eq!(run["commands"], 0);
    assert!(decisions(run).iter().all(|(_, _, d)| d == "all_off"));

    // Its on lights its lights at the current values and resumes the adjustment.
    commands.lock().unwrap().clear();
    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/labels/{}/on", ambient["id"]),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((&body["action"], &body["on"]), (&json!("on"), &json!(true)));
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["runs"][0]["trigger"], "label_on");
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

    // "All off" and "all on", until the app moves to labels, do the same to every label.
    assert_eq!(
        call(&app, "POST", "/api/lights/off", None).await.0,
        StatusCode::OK
    );
    commands.lock().unwrap().clear();
    home.adjust(NIGHT + 600, "scheduled").await;
    assert_eq!(tunings(&commands), []);
    drop((home, app));
    let home = home_link::Home::new(home_link::open_db(db_path).unwrap(), url);
    let app = home.router();
    home.adjust(NIGHT + 1200, "scheduled").await;
    assert_eq!(tunings(&commands), []);
    assert_eq!(
        call(&app, "POST", "/api/lights/on", None).await.0,
        StatusCode::OK
    );
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["runs"][0]["trigger"], "lights_on");
    assert_eq!(body["labels"][0]["on"], true);
    assert_eq!(tunings(&commands).len(), 6);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn all_on_writes_the_current_values_to_lights_whose_on_is_reported_late() {
    // The T2 behind the bridge accepts On but is still reported off.
    let fake = Fake {
        late_on: vec![(1, 3)],
        ..Fake::default()
    };
    let (addr, commands, _) = fake_light_server(tuning_nodes(), fake).await;
    let (home, app, _) = auto_home(addr, None).await;
    enable(&app).await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .cast_signed();
    // The current values were already sent while it was on.
    home.adjust(now, "scheduled").await;
    assert_eq!(
        call(&app, "POST", "/api/lights/off", None).await.0,
        StatusCode::OK
    );

    commands.lock().unwrap().clear();
    let (status, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["lights"][1]["result"], "switched");
    // Switched on in this request, it is not read again but written at once,
    // even if off, with the values sent before.
    assert!(!reads(&commands).contains(&(json!(1), json!(["3/6/0"]))));
    let (_, schedule) = call(&app, "GET", "/api/lights/schedule", None).await;
    let run = &schedule["runs"][0];
    assert_eq!(run["trigger"], "lights_on");
    assert_eq!(run["lights"][1]["decision"], "sent");
    let (level, mireds) = (&run["lights"][1]["level"], &run["lights"][1]["mireds"]);
    let t2: Vec<_> = tunings(&commands)
        .into_iter()
        .filter(|(node, endpoint, ..)| (node, endpoint) == (&json!(1), &json!(3)))
        .map(|(_, _, cluster, command, payload)| (cluster, command, payload))
        .collect();
    assert_eq!(
        t2,
        [
            (
                json!(8),
                json!("MoveToLevelWithOnOff"),
                json!({ "level": level, "transitionTime": 0, "optionsMask": 1, "optionsOverride": 1 })
            ),
            (
                json!(768),
                json!("MoveToColorTemperature"),
                json!({ "colorTemperatureMireds": mireds, "transitionTime": 0, "optionsMask": 1, "optionsOverride": 1 })
            ),
        ]
    );
}

/// 00:30 in Tokyo on 2026-10-06, before sunrise.
const AFTER_MIDNIGHT: i64 = NIGHT + 90 * 60;

#[tokio::test]
async fn after_midnight_the_schedule_and_all_on_write_half_the_night_level() {
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let (home, app, _) = auto_home(addr, Some(|| AFTER_MIDNIGHT)).await;
    enable(&app).await;

    home.adjust(AFTER_MIDNIGHT, "scheduled").await;
    // The lights read as on get half the night's level and its usual colour.
    let tuned: Vec<_> = tunings(&commands)
        .into_iter()
        .map(|(node, _, _, command, payload)| (node, command, payload))
        .collect();
    assert_eq!(
        tuned,
        [
            (json!(1), json!("MoveToLevel"), level(51)),
            (json!(1), json!("MoveToColorTemperature"), mireds(333)),
            (json!(5), json!("MoveToLevel"), level(51)),
            (json!(5), json!("MoveToColorTemperature"), mireds(333)),
        ]
    );
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    let run = &body["runs"][0];
    assert_eq!(run["at"], "2026-10-06T00:30:00+09:00");
    assert_eq!(run["level"], 51);
    assert_eq!(run["lights"][1]["target_level"], 51);

    // "All on" after midnight lights them at half the night's level too.
    commands.lock().unwrap().clear();
    assert_eq!(
        call(&app, "POST", "/api/lights/on", None).await.0,
        StatusCode::OK
    );
    let levels: Vec<_> = tunings(&commands)
        .into_iter()
        .filter(|(_, _, cluster, ..)| cluster == &json!(8))
        .map(|(node, _, _, command, payload)| (node, command, payload["level"].clone()))
        .collect();
    assert_eq!(
        levels,
        [
            (json!(1), json!("MoveToLevelWithOnOff"), json!(51)),
            (json!(5), json!("MoveToLevelWithOnOff"), json!(51)),
            (json!(6), json!("MoveToLevelWithOnOff"), json!(51)),
        ]
    );
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["runs"][0]["trigger"], "lights_on");
    assert_eq!(body["runs"][0]["at"], "2026-10-06T00:30:00+09:00");
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
                "warm_kelvin": 3000, "cool_kelvin": 5000, "late_night_percent": 50 })
    );
    assert!(body.get("intent").is_none(), "{body}");
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
        json!({ "late_night_percent": 0 }),
        json!({ "late_night_percent": 101 }),
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

/// Server info as matterjs-server greets a connection, with Bluetooth and a
/// Thread dataset set or not.
fn server_info(bluetooth: bool, thread: bool) -> Value {
    json!({ "fabric_id": 1, "schema_version": 13, "wifi_credentials_set": false,
            "thread_credentials_set": thread, "bluetooth_enabled": bluetooth })
}

/// Serves `info`, `set_wifi_credentials`, `commission_with_code` and
/// `read_attribute` the way matterjs-server does, recording every command.
async fn fake_commissioner(
    info: Value,
    commission: Commission,
    read: Value,
) -> (SocketAddr, Arc<Mutex<Vec<Value>>>) {
    let (addr, commands, _) = fake_matter(info, commission, read).await;
    (addr, commands)
}

/// `fake_commissioner` that also answers `get_nodes` with its nodes, which a
/// commissioned node joins; the test may change them.
async fn fake_matter(
    info: Value,
    commission: Commission,
    read: Value,
) -> (SocketAddr, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<Value>>>) {
    let nodes = Arc::new(Mutex::new(Vec::new()));
    let served = nodes.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let seen = commands.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::text(info.to_string())).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let command: Value = serde_json::from_str(&text).unwrap();
                seen.lock().unwrap().push(command.clone());
                let id = command["message_id"].clone();
                let reply = match (command["command"].as_str().unwrap(), &commission) {
                    ("set_wifi_credentials" | "remove_wifi_credentials", _) => {
                        json!({ "message_id": id, "result": {} })
                    }
                    ("get_nodes", _) => {
                        json!({ "message_id": id, "result": served.lock().unwrap().clone() })
                    }
                    ("commission_with_code", Commission::Node(node)) => {
                        served.lock().unwrap().push(node.clone());
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
    (addr, commands, nodes)
}

const PASSWORD: &str = "kakushi-pass-7f3a";

fn commission_body() -> Value {
    json!({ "qr_payload": QR, "name": "押入れ1" })
}

/// A Tapo bulb as matterjs-server returns it right after commissioning.
fn tapo_node() -> Value {
    json!({ "node_id": 17, "available": true, "attributes": {
        "0/40/1": "Tapo", "0/40/15": TAPO, "0/40/3": "Smart Bulb",
        "0/51/0": [{ "0": "wlan0", "4": "qrvM3e7/", "7": 1 }],
    }})
}

/// An app on `db` with "home-2g" saved as the Wi-Fi for devices.
fn commissioning_app(addr: SocketAddr, db: rusqlite::Connection) -> Router {
    db.execute(
        "INSERT INTO wifi_networks (ssid, password, for_devices) VALUES ('home-2g', ?1, 1)",
        [PASSWORD],
    )
    .unwrap();
    wifiless_app(addr, db)
}

fn wifiless_app(addr: SocketAddr, db: rusqlite::Connection) -> Router {
    home_link::Home::new(db, Some(format!("ws://{addr}/ws")))
        .with_commission_timeout(std::time::Duration::from_millis(500))
        .router()
}

const IOT_PASSWORD: &str = "iot-himitsu-91c2";

#[tokio::test]
async fn wifi_networks_are_saved_listed_chosen_and_deleted_without_their_passwords() {
    let (logs, _guard) = capture_logs();
    let app = app();
    let (status, list) = call(&app, "GET", "/api/wifi", None).await;
    assert_eq!((status, list), (StatusCode::OK, json!([])));

    // The first network saved is the one handed to devices.
    let (status, home) = call(
        &app,
        "POST",
        "/api/wifi",
        Some(json!({ "ssid": "home-2g", "password": PASSWORD })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{home}");
    assert_eq!(
        (&home["ssid"], &home["for_devices"]),
        (&json!("home-2g"), &json!(true))
    );
    let (status, iot) = call(
        &app,
        "POST",
        "/api/wifi",
        Some(json!({ "ssid": "iot-2g", "password": IOT_PASSWORD })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{iot}");
    assert_eq!(iot["for_devices"], false);
    let ids = |list: &Value| -> Vec<(Value, Value)> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|w| (w["ssid"].clone(), w["for_devices"].clone()))
            .collect()
    };

    let uri = format!("/api/wifi/{}", iot["id"]);
    let (status, chosen) = call(
        &app,
        "PUT",
        "/api/wifi/devices",
        Some(json!({ "id": iot["id"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{chosen}");
    assert_eq!(chosen["for_devices"], true);
    let (_, list) = call(&app, "GET", "/api/wifi", None).await;
    assert_eq!(
        ids(&list),
        [
            (json!("home-2g"), json!(false)),
            (json!("iot-2g"), json!(true))
        ]
    );

    // The same name replaces the password in place and keeps the choice.
    let (status, again) = call(
        &app,
        "POST",
        "/api/wifi",
        Some(json!({ "ssid": "home-2g", "password": "kawatta-pass-55" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        (&again["id"], &again["for_devices"]),
        (&home["id"], &json!(false))
    );
    let (_, list) = call(&app, "GET", "/api/wifi", None).await;
    assert_eq!(list.as_array().unwrap().len(), 2);

    for body in [
        json!({ "ssid": "", "password": PASSWORD }),
        json!({ "ssid": "x".repeat(33), "password": PASSWORD }),
        json!({ "ssid": "home-2g", "password": "" }),
        json!({ "ssid": "home-2g", "password": "x".repeat(65) }),
    ] {
        let (status, error) = call(&app, "POST", "/api/wifi", Some(body)).await;
        assert_eq!(
            (status, error["error"].as_str().unwrap()),
            (StatusCode::BAD_REQUEST, "invalid_wifi")
        );
    }
    for (method, uri, body) in [
        ("PUT", "/api/wifi/devices", Some(json!({ "id": 999 }))),
        ("DELETE", "/api/wifi/999", None),
    ] {
        let (status, error) = call(&app, method, uri, body).await;
        assert_eq!(
            (status, error["error"].as_str().unwrap()),
            (StatusCode::NOT_FOUND, "wifi_not_found")
        );
    }

    // Deleting the network for devices hands devices the one left.
    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = call(&app, "GET", "/api/wifi", None).await;
    assert_eq!(ids(&list), [(json!("home-2g"), json!(true))]);

    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("/api/wifi"), "logs were not captured: {logs}");
    for secret in [PASSWORD, IOT_PASSWORD, "kawatta-pass-55"] {
        assert!(!list.to_string().contains(secret));
        assert!(!home.to_string().contains(secret) && !again.to_string().contains(secret));
        assert!(!logs.contains(secret), "password in logs: {logs}");
    }
}

#[tokio::test]
async fn commissioning_hands_over_the_wifi_for_devices_then_registers_the_new_node() {
    let (logs, _guard) = capture_logs();
    let (addr, commands) = fake_commissioner(
        server_info(true, false),
        Commission::Node(tapo_node()),
        json!({}),
    )
    .await;
    let app = wifiless_app(addr, home_link::open_db(":memory:").unwrap());
    for (ssid, password) in [("home-2g", PASSWORD), ("iot-2g", IOT_PASSWORD)] {
        let body = json!({ "ssid": ssid, "password": password, "for_devices": true });
        let (status, _) = call(&app, "POST", "/api/wifi", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // The phone sends no Wi-Fi; any it sends is ignored.
    let mut body = commission_body();
    body["wifi_ssid"] = json!("phone-wifi");
    body["wifi_password"] = json!("phone-pass-0000");
    let (status, body) = call(&app, "POST", "/api/commission", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["node_id"], 17);
    assert_eq!(body["registered"], true);
    let device = &body["device"];
    assert_eq!(
        (
            &device["name"],
            &device["vendor"],
            &device["serial_number"],
            &device["mac"]
        ),
        (
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
        [
            json!("set_wifi_credentials"),
            json!("commission_with_code"),
            json!("remove_wifi_credentials")
        ]
    );
    assert_eq!(
        commands[0]["args"],
        json!({ "ssid": "iot-2g", "credentials": IOT_PASSWORD })
    );
    assert_eq!(
        commands[1]["args"],
        json!({ "code": QR, "network_only": false })
    );

    let id = device["id"].as_i64().unwrap();
    let (_, stored) = call(&app, "GET", &format!("/api/devices/{id}"), None).await;
    assert_eq!(stored["qr_payload"], QR);
    let (_, all) = call(&app, "GET", "/api/devices", None).await;
    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(
        logs.contains("device commissioned"),
        "logs were not captured: {logs}"
    );
    for secret in [PASSWORD, IOT_PASSWORD] {
        assert!(!body.to_string().contains(secret));
        assert!(!all.to_string().contains(secret));
        assert!(!logs.contains(secret), "password in logs: {logs}");
    }
}

#[tokio::test]
async fn commissioning_onto_wifi_needs_a_wifi_for_devices() {
    let (addr, commands) = fake_commissioner(
        server_info(true, false),
        Commission::Node(tapo_node()),
        json!({}),
    )
    .await;
    let app = wifiless_app(addr, home_link::open_db(":memory:").unwrap());
    let (status, error) = call(&app, "POST", "/api/commission", Some(commission_body())).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (StatusCode::CONFLICT, "wifi_not_set")
    );
    assert_eq!(commands.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn commissioning_reads_identifiers_missing_from_the_new_node() {
    let node = json!({ "node_id": 18, "attributes": {} });
    let read = json!({ "0/40/1": "Uascent", "0/40/15": "U2025", "0/51/0": [] });
    let (addr, commands) =
        fake_commissioner(server_info(true, false), Commission::Node(node), read).await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let body = json!({ "manual_code": "3497-011-2332" });
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
        let (addr, _) = fake_commissioner(server_info(true, false), commission, json!({})).await;
        let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
        let (got, error) = call(&app, "POST", "/api/commission", Some(commission_body())).await;
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
    let (addr, _) = fake_commissioner(
        server_info(true, false),
        Commission::Node(tapo_node()),
        json!({}),
    )
    .await;
    let (db, sql) = ledger();
    let app = commissioning_app(addr, db);
    // Recorded by its code before, without identifiers.
    let old = add_device(
        &app,
        &sql,
        json!({ "manual_code": MANUAL, "name": "読書灯" }),
    )
    .await;

    let (status, body) = call(&app, "POST", "/api/commission", Some(commission_body())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["registered"], false);
    assert_eq!(body["device"]["id"], old["id"]);
    assert_eq!(body["device"]["name"], "読書灯");
    // The entry gains the identifiers it lacked.
    assert_eq!(body["device"]["serial_number"], TAPO);
    assert_eq!(body["device"]["mac"], TAPO);
    let (_, devices) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(devices.as_array().unwrap().len(), 1);

    // Again, now found by its identifiers.
    let (status, body) = call(&app, "POST", "/api/commission", Some(commission_body())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["device"]["id"], old["id"]);
}

#[tokio::test]
async fn commissioning_checks_its_input_before_reaching_the_matter_server() {
    let (addr, commands) = fake_commissioner(
        server_info(true, false),
        Commission::Node(tapo_node()),
        json!({}),
    )
    .await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let with = |field: &str, value: Value| {
        let mut body = commission_body();
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
        // A manual code is not accepted as a QR payload.
        (
            with("qr_payload", json!(MANUAL)),
            StatusCode::BAD_REQUEST,
            "invalid_qr_payload",
        ),
        (
            json!({ "manual_code": "34970112331", "name": "押入れ1" }),
            StatusCode::BAD_REQUEST,
            "invalid_manual_code",
        ),
        (
            with("manual_code", json!(MANUAL)),
            StatusCode::BAD_REQUEST,
            "missing_setup_code",
        ),
        (
            with("name", json!("x".repeat(101))),
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
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
    let (addr, commands) = fake_commissioner(
        server_info(false, true),
        Commission::Node(tapo_node()),
        json!({}),
    )
    .await;
    let ble_off = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let (status, error) = call(&ble_off, "POST", "/api/commission", Some(commission_body())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"], "bluetooth_unavailable");
    assert_eq!(commands.lock().unwrap().len(), 0);

    let unset = app();
    let (status, error) = call(&unset, "POST", "/api/commission", Some(commission_body())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"], "matter_server_not_configured");

    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let app = commissioning_app(closed, home_link::open_db(":memory:").unwrap());
    let (status, error) = call(&app, "POST", "/api/commission", Some(commission_body())).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(error["error"], "matter_server_unreachable");
}

/// A T2 bulb as matterjs-server returns it after commissioning onto Thread.
fn t2_node() -> Value {
    json!({ "node_id": 21, "available": true, "attributes": {
        "0/40/1": "Aqara", "0/40/15": "T2-0001", "0/40/3": "Light Bulb T2",
        "0/51/0": [{ "0": "thread0", "4": "AAECAwQFBgc=", "7": 4 }],
    }})
}

fn thread_body() -> Value {
    json!({ "network": "thread", "manual_code": MANUAL, "name": "寝室T2" })
}

#[tokio::test]
async fn thread_commissioning_uses_the_stored_dataset_and_registers_the_node() {
    let (addr, commands) = fake_commissioner(
        server_info(true, true),
        Commission::Node(t2_node()),
        json!({}),
    )
    .await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());

    let (status, body) = call(&app, "POST", "/api/commission", Some(thread_body())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["node_id"], 21);
    let device = &body["device"];
    assert_eq!(
        (
            &device["name"],
            &device["vendor"],
            &device["serial_number"],
            &device["mac"]
        ),
        (
            &json!("寝室T2"),
            &json!("Aqara"),
            &json!("T2-0001"),
            &Value::Null
        )
    );
    // No Wi-Fi is handed over: matterjs-server commissions with its own dataset.
    let commands = commands.lock().unwrap().clone();
    assert_eq!(
        commands.iter().map(|c| &c["command"]).collect::<Vec<_>>(),
        ["commission_with_code", "remove_wifi_credentials"]
    );
    assert_eq!(
        commands[0]["args"],
        json!({ "code": MANUAL, "network_only": false })
    );
}

#[tokio::test]
async fn thread_commissioning_is_refused_until_the_thread_network_is_ready() {
    let (addr, commands) = fake_commissioner(
        server_info(true, false),
        Commission::Node(t2_node()),
        json!({}),
    )
    .await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let (status, error) = call(&app, "POST", "/api/commission", Some(thread_body())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"], "thread_not_ready");
    assert_eq!(error["message"], "Thread網が未準備です");
    assert_eq!(commands.lock().unwrap().len(), 0);
    let (_, devices) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(devices, json!([]));
}

#[tokio::test]
async fn thread_commissioning_checks_the_network_and_ignores_wifi_fields() {
    let (addr, commands) = fake_commissioner(
        server_info(true, true),
        Commission::Node(t2_node()),
        json!({}),
    )
    .await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let mut body = thread_body();
    body["network"] = json!("zigbee");
    let (status, error) = call(&app, "POST", "/api/commission", Some(body)).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (StatusCode::BAD_REQUEST, "invalid_network")
    );
    assert_eq!(commands.lock().unwrap().len(), 0);

    // Wi-Fi stays the default and needs a Wi-Fi for devices.
    let app = wifiless_app(addr, home_link::open_db(":memory:").unwrap());
    let mut body = thread_body();
    body.as_object_mut().unwrap().remove("network");
    let (status, error) = call(&app, "POST", "/api/commission", Some(body)).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (StatusCode::CONFLICT, "wifi_not_set")
    );
}

#[tokio::test]
async fn thread_failures_are_told_apart() {
    let (addr, _) = fake_commissioner(
        server_info(true, true),
        Commission::Error("Commission failed: Commissionee failed to connect to Thread network \"home\": NetworkNotFound"),
        json!({}),
    )
    .await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let (status, error) = call(&app, "POST", "/api/commission", Some(thread_body())).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (StatusCode::UNPROCESSABLE_ENTITY, "thread_failed")
    );
}

#[tokio::test]
async fn thread_readiness_follows_the_matter_server() {
    for thread in [true, false] {
        let (addr, commands) =
            fake_commissioner(server_info(true, thread), Commission::Silent, json!({})).await;
        let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
        let (status, body) = call(&app, "GET", "/api/thread", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "ready": thread }));
        assert_eq!(commands.lock().unwrap().len(), 0);
    }
    let (status, error) = call(&app(), "GET", "/api/thread", None).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "matter_server_not_configured"
        )
    );
    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let app = commissioning_app(closed, home_link::open_db(":memory:").unwrap());
    let (status, error) = call(&app, "GET", "/api/thread", None).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (StatusCode::BAD_GATEWAY, "matter_server_unreachable")
    );
}

// Three made-up devices of the test vendor 0xFFF1, products 0x8001 to 0x8003.
const QRS: [&str; 3] = [
    "MT:-24J042C00PE0648G00",
    "MT:06PS042C00GV0648G00",
    "MT:2980142C007A1648G00",
];

async fn scan(app: &Router, body: Value) -> (StatusCode, Value) {
    call(app, "POST", "/api/devices", Some(body)).await
}

async fn connect(app: &Router, id: &Value) -> (StatusCode, Value) {
    call(app, "POST", &format!("/api/devices/{id}/commission"), None).await
}

fn states(list: &Value) -> Vec<Value> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|d| d["state"].clone())
        .collect()
}

/// The commands sent to matterjs-server but reading its nodes.
fn sent(commands: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c["command"] != "get_nodes")
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_code_alone_enters_the_ledger_once() {
    let (addr, commands, _) =
        fake_matter(server_info(true, false), Commission::Silent, json!({})).await;
    let app = wifiless_app(addr, home_link::open_db(":memory:").unwrap());

    let (status, body) = scan(&app, json!({ "qr_payload": QRS[0] })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["registered"], true);
    let device = &body["device"];
    assert_eq!(
        (
            &device["name"],
            &device["state"],
            &device["vendor_id"],
            &device["product_id"],
        ),
        (
            &json!(""),
            &json!("unconnected"),
            &json!(0xFFF1),
            &json!(0x8001),
        )
    );
    let (status, again) = scan(&app, json!({ "qr_payload": format!(" {} ", QRS[0]) })).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        (&again["registered"], &again["device"]["id"]),
        (&json!(false), &device["id"])
    );

    // A manual code carries no vendor or product; the device's QR adds them.
    let (status, manual) = scan(&app, json!({ "manual_code": "3497-0112-332" })).await;
    assert_eq!(status, StatusCode::CREATED, "{manual}");
    assert_eq!(
        (
            &manual["device"]["vendor_id"],
            &manual["device"]["product_id"]
        ),
        (&Value::Null, &Value::Null)
    );
    let (status, qr) = scan(&app, json!({ "qr_payload": QR })).await;
    assert_eq!(status, StatusCode::OK, "{qr}");
    assert_eq!(qr["device"]["id"], manual["device"]["id"]);
    assert_eq!(
        (&qr["device"]["vendor_id"], &qr["device"]["product_id"]),
        (&json!(0xFFF1), &json!(0x8000))
    );
    let (_, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert!(!list.to_string().contains("042C00"), "{list}");
    let id = manual["device"]["id"].as_i64().unwrap();
    let (_, stored) = call(&app, "GET", &format!("/api/devices/{id}"), None).await;
    assert_eq!(stored["manual_code"], MANUAL);

    for (body, code) in [
        (json!({}), "missing_setup_code"),
        (
            json!({ "qr_payload": "https://example.com" }),
            "invalid_qr_payload",
        ),
        (
            json!({ "manual_code": "34970112331" }),
            "invalid_manual_code",
        ),
    ] {
        let (status, error) = scan(&app, body).await;
        assert_eq!(
            (status, error["error"].as_str().unwrap()),
            (StatusCode::BAD_REQUEST, code)
        );
    }
    // Recording only reads matterjs-server's nodes.
    assert_eq!(sent(&commands), Vec::<Value>::new());
}

#[tokio::test]
async fn three_codes_scanned_first_then_connecting_one_connects_only_that_row() {
    let (logs, _guard) = capture_logs();
    let (addr, commands, nodes) = fake_matter(
        server_info(true, false),
        Commission::Node(tapo_node()),
        json!({}),
    )
    .await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let mut ids = Vec::new();
    for qr in QRS {
        let (status, body) = scan(&app, json!({ "qr_payload": qr })).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        ids.push(body["device"]["id"].clone());
    }
    let (_, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(states(&list), vec![json!("unconnected"); 3]);
    // Devices never connected are not lights gone missing.
    let (status, lights) = call(&app, "GET", "/api/lights", None).await;
    assert_eq!(status, StatusCode::OK, "{lights}");
    assert_eq!(lights["missing"], 0);

    let (status, body) = connect(&app, &ids[1]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["node_id"], 17);
    let device = &body["device"];
    assert_eq!(
        (
            &device["id"],
            &device["state"],
            &device["vendor"],
            &device["serial_number"],
            &device["mac"],
            &device["product_name"],
        ),
        (
            &ids[1],
            &json!("connected"),
            &json!("Tapo"),
            &json!(TAPO),
            &json!(TAPO),
            &json!("Smart Bulb"),
        )
    );
    let commands = sent(&commands);
    assert_eq!(
        commands.iter().map(|c| &c["command"]).collect::<Vec<_>>(),
        [
            "set_wifi_credentials",
            "commission_with_code",
            "remove_wifi_credentials"
        ]
    );
    assert_eq!(
        commands[0]["args"],
        json!({ "ssid": "home-2g", "credentials": PASSWORD })
    );
    assert_eq!(
        commands[1]["args"],
        json!({ "code": QRS[1], "network_only": false })
    );
    let (_, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(
        states(&list),
        [
            json!("unconnected"),
            json!("connected"),
            json!("unconnected")
        ]
    );

    // The state follows matterjs-server's nodes.
    let uri = format!("/api/devices/{}", ids[1]);
    nodes.lock().unwrap()[0]["available"] = json!(false);
    let (_, device) = call(&app, "GET", &uri, None).await;
    assert_eq!(device["state"], "no_response");
    nodes.lock().unwrap().clear();
    let (_, device) = call(&app, "GET", &uri, None).await;
    assert_eq!(
        (&device["state"], &device["product_name"]),
        (&json!("unconnected"), &Value::Null)
    );
    let (_, lights) = call(&app, "GET", "/api/lights", None).await;
    assert_eq!(lights["missing"], 1);

    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("device commissioned"), "{logs}");
    for text in [body.to_string(), list.to_string(), logs] {
        assert!(!text.contains(PASSWORD), "{text}");
    }
}

#[tokio::test]
async fn connecting_a_row_without_a_saved_wifi_leaves_the_network_to_the_device() {
    let (addr, commands, _) = fake_matter(
        server_info(true, true),
        Commission::Node(t2_node()),
        json!({}),
    )
    .await;
    let app = wifiless_app(addr, home_link::open_db(":memory:").unwrap());
    let (_, body) = scan(&app, json!({ "manual_code": MANUAL })).await;
    let (status, body) = connect(&app, &body["device"]["id"]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            &body["device"]["serial_number"],
            &body["device"]["mac"],
            &body["device"]["state"]
        ),
        (&json!("T2-0001"), &Value::Null, &json!("connected"))
    );
    assert_eq!(
        sent(&commands),
        [
            json!({ "message_id": "commission", "command": "commission_with_code",
                 "args": { "code": MANUAL, "network_only": false } }),
            json!({ "message_id": "forget", "command": "remove_wifi_credentials" })
        ]
    );
}

#[tokio::test]
async fn connecting_a_row_tells_failures_apart_and_leaves_it_unconnected() {
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
                "Commission failed: Commissionee failed to connect to Thread network \"home\": NetworkNotFound",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "thread_failed",
        ),
        (
            Commission::Silent,
            StatusCode::GATEWAY_TIMEOUT,
            "commission_timeout",
        ),
        (
            Commission::Error(
                "Commission failed: commissioning discovery failed: No device could be commissioned (1 of 1 started attempt(s) failed, 1 discovered)\nCause #0: [ble] No connected BLE proxy client owns peripheral 00:00:5E:00:53:01",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "commission_failed",
        ),
    ];
    for (commission, status, code) in cases {
        let (addr, _, _) = fake_matter(server_info(true, false), commission, json!({})).await;
        let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
        let (_, body) = scan(&app, json!({ "qr_payload": QR })).await;
        let id = &body["device"]["id"];
        let (got, error) = connect(&app, id).await;
        assert_eq!(
            (got, error["error"].as_str().unwrap()),
            (status, code),
            "{error}"
        );
        assert!(!error.to_string().contains(PASSWORD));
        let (_, device) = call(&app, "GET", &format!("/api/devices/{id}"), None).await;
        assert_eq!(
            (&device["serial_number"], &device["state"]),
            (&Value::Null, &json!("unconnected")),
            "{code}"
        );
    }

    let (addr, commands, _) = fake_matter(
        server_info(false, false),
        Commission::Node(tapo_node()),
        json!({}),
    )
    .await;
    let app = commissioning_app(addr, home_link::open_db(":memory:").unwrap());
    let (_, body) = scan(&app, json!({ "qr_payload": QR })).await;
    let (status, error) = connect(&app, &body["device"]["id"]).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (StatusCode::SERVICE_UNAVAILABLE, "bluetooth_unavailable")
    );
    let (status, error) = connect(&app, &json!(999)).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (StatusCode::NOT_FOUND, "device_not_found")
    );
    assert_eq!(sent(&commands), Vec::<Value>::new());

    let unset = home_link::app(home_link::open_db(":memory:").unwrap(), None);
    let (_, body) = scan(&unset, json!({ "qr_payload": QR })).await;
    assert_eq!(body["device"]["state"], Value::Null);
    let (status, error) = connect(&unset, &body["device"]["id"]).await;
    assert_eq!(
        (status, error["error"].as_str().unwrap()),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "matter_server_not_configured"
        )
    );
}

#[tokio::test]
async fn connecting_a_device_the_ledger_already_holds_keeps_that_row() {
    let (addr, _, _) = fake_matter(
        server_info(true, false),
        Commission::Node(tapo_node()),
        json!({}),
    )
    .await;
    let (db, sql) = ledger();
    let app = commissioning_app(addr, db);
    let old = add_device(
        &app,
        &sql,
        json!({ "vendor": "Tapo", "serial_number": TAPO, "name": "読書灯" }),
    )
    .await;
    let label = create_label(&app, json!({ "name": "夜" })).await;
    assign(&app, &old, &[&label]).await;
    ignore(&app, &[old["id"].as_i64().unwrap()]).await;

    // Reset and scanned again, the device is connected onto its old row.
    let (_, scanned) = scan(&app, json!({ "qr_payload": QR })).await;
    let (status, body) = connect(&app, &scanned["device"]["id"]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let device = &body["device"];
    assert_eq!(
        (
            &device["id"],
            &device["name"],
            &device["ignored"],
            &device["labels"][0]["name"],
            &device["state"],
        ),
        (
            &old["id"],
            &json!("読書灯"),
            &json!(true),
            &json!("夜"),
            &json!("connected"),
        )
    );
    let (_, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, stored) = call(&app, "GET", &format!("/api/devices/{}", old["id"]), None).await;
    assert_eq!(stored["qr_payload"], QR);
}

#[tokio::test]
async fn devices_are_named_by_patch() {
    let (app, _) = ledger_app();
    let (_, body) = scan(&app, json!({ "qr_payload": QR })).await;
    let uri = format!("/api/devices/{}", body["device"]["id"]);

    let (status, device) = call(&app, "PATCH", &uri, Some(json!({ "name": " 押入れ1 " }))).await;
    assert_eq!(status, StatusCode::OK, "{device}");
    assert_eq!(device["name"], "押入れ1");
    let (status, device) = call(&app, "PATCH", &uri, Some(json!({ "name": "" }))).await;
    assert_eq!(status, StatusCode::OK, "{device}");

    for (body, status, code) in [
        (
            json!({ "name": "x".repeat(101) }),
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
        ),
        (
            json!({ "name": null }),
            StatusCode::BAD_REQUEST,
            "invalid_device_update",
        ),
        (
            json!({ "room_id": 1 }),
            StatusCode::BAD_REQUEST,
            "invalid_device_update",
        ),
        (json!({}), StatusCode::BAD_REQUEST, "invalid_device_update"),
        (
            json!({ "name": "y", "color": 1 }),
            StatusCode::BAD_REQUEST,
            "invalid_device_update",
        ),
    ] {
        let (got, error) = call(&app, "PATCH", &uri, Some(body.clone())).await;
        assert_eq!(
            (got, error["error"].as_str().unwrap()),
            (status, code),
            "{body}"
        );
    }
    let (_, device) = call(&app, "GET", &uri, None).await;
    assert_eq!(device["name"], "");
}

#[tokio::test]
async fn opening_a_v0_1_30_ledger_keeps_its_lights_connected_with_their_labels() {
    let path = std::env::temp_dir().join(format!("home-link-v6-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        // The schema of v0.1.30 (user_version 6).
        let old = rusqlite::Connection::open(&path).unwrap();
        old.execute_batch(&format!(
            "CREATE TABLE rooms (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE labels (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                day_level INTEGER,
                night_level INTEGER,
                cool_kelvin INTEGER,
                warm_kelvin INTEGER
            , boost TEXT, dedicated INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE device_labels (
                device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
                label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
                PRIMARY KEY (device_id, label_id)
            );
            CREATE TABLE \"devices\" (
                id INTEGER PRIMARY KEY,
                room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
                qr_payload TEXT UNIQUE,
                vendor TEXT,
                serial_number TEXT,
                mac TEXT UNIQUE,
                name TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ignored INTEGER NOT NULL DEFAULT 0,
                UNIQUE (vendor, serial_number),
                CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
            );
            PRAGMA user_version = 6;
            INSERT INTO rooms (name) VALUES ('リビング');
            INSERT INTO labels (name, warm_kelvin) VALUES ('夜', 2700);
            INSERT INTO devices (id, room_id, qr_payload, vendor, serial_number, mac, name, ignored) VALUES
                (3, 1, '{MANUAL}', 'Aqara', 'SERIAL-A', NULL, 'キッチン1', 0),
                (4, 1, '{QR}', 'Tapo', '{TAPO}', '{TAPO}', '書斎', 1);
            INSERT INTO device_labels VALUES (3, 1);"
        ))
        .unwrap();
    }
    for _ in 0..2 {
        let db = home_link::open_db(path.to_str().unwrap()).unwrap();
        let rows: Vec<Value> = db
            .prepare(
                "SELECT json_array(devices.id, name, ignored, vendor_id, product_id)
                 FROM devices ORDER BY devices.id",
            )
            .unwrap()
            .query_map([], |r| {
                Ok(serde_json::from_str(&r.get::<_, String>(0)?).unwrap())
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                json!([3, "キッチン1", 0, null, null]),
                json!([4, "書斎", 1, 0xFFF1, 0x8000]),
            ]
        );
        assert_eq!(assignments(&db), [(3, vec![1, 2]), (4, vec![2])]);
    }

    let (addr, _, nodes) =
        fake_matter(server_info(true, false), Commission::Silent, json!({})).await;
    nodes.lock().unwrap().extend([
        json!({ "node_id": 2, "available": true, "attributes": {
            "0/40/1": "Aqara", "0/40/15": "SERIAL-A", "0/40/3": "Light Bulb T2" }}),
        tapo_node(),
    ]);
    let app = wifiless_app(addr, home_link::open_db(path.to_str().unwrap()).unwrap());
    let (_, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(states(&list), vec![json!("connected"); 2]);
    assert_eq!(
        (&list[0]["labels"][0]["name"], &list[1]["ignored"]),
        (&json!("夜"), &json!(true))
    );
    let (status, body) = scan(&app, json!({ "qr_payload": QRS[0] })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn devices_are_ignored_and_unignored_by_patch() {
    let (app, sql) = ledger_app();
    let (_, label) = call(&app, "POST", "/api/labels", Some(json!({ "name": "夜" }))).await;
    let device = add_device(
        &app,
        &sql,
        json!({ "vendor": "Aqara", "serial_number": "54ef44100126e0a6" }),
    )
    .await;
    assert_eq!(device["ignored"], false);
    let uri = format!("/api/devices/{}", device["id"]);
    call(
        &app,
        "PATCH",
        &uri,
        Some(json!({ "label_ids": [label["id"]] })),
    )
    .await;
    let labelled = json!([{ "id": label["id"], "name": "夜" }]);

    // Ignoring keeps the label; the label alone keeps the ignoring.
    let (status, device) = call(&app, "PATCH", &uri, Some(json!({ "ignored": true }))).await;
    assert_eq!(status, StatusCode::OK, "{device}");
    assert_eq!(
        (&device["ignored"], &device["labels"]),
        (&json!(true), &labelled)
    );
    let (_, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(list[0]["ignored"], true);
    let (_, device) = call(&app, "PATCH", &uri, Some(json!({ "label_ids": [] }))).await;
    assert_eq!(
        (&device["ignored"], &device["labels"]),
        (&json!(true), &json!([]))
    );
    let (status, device) = call(
        &app,
        "PATCH",
        &uri,
        Some(json!({ "ignored": false, "label_ids": [label["id"]] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{device}");
    assert_eq!(
        (&device["ignored"], &device["labels"]),
        (&json!(false), &labelled)
    );

    for bad in [
        json!({ "ignored": null }),
        json!({ "ignored": "true" }),
        json!({ "ignored": 1 }),
        json!({ "ignored": true, "muted": true }),
        json!([true]),
    ] {
        let (status, error) = call(&app, "PATCH", &uri, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(error["error"], "invalid_device_update", "{bad}");
    }
    let (_, device) = call(&app, "GET", &uri, None).await;
    assert_eq!(device["ignored"], false);
    let (status, _) = call(
        &app,
        "PATCH",
        "/api/devices/999",
        Some(json!({ "ignored": true })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Ignores the ledger devices at `ids`.
async fn ignore(app: &Router, ids: &[i64]) {
    for id in ids {
        let (status, body) = call(
            app,
            "PATCH",
            &format!("/api/devices/{id}"),
            Some(json!({ "ignored": true })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

#[tokio::test]
async fn ignored_lights_get_no_switch_and_are_not_counted() {
    let (addr, commands, _) = fake_light_server(home_nodes(), Fake::default()).await;
    let (app, sql) = light_app(addr).await;
    // The unreachable T2 behind the bridge, the reachable Tapo (読書灯, id 1) and
    // a ledger device matterjs-server does not serve.
    let mut ids = vec![1];
    for serial in ["t2-a", "gone-for-good"] {
        let vendor = if serial == "t2-a" { "Aqara" } else { "Tapo" };
        let device = add_device(
            &app,
            &sql,
            json!({ "vendor": vendor, "serial_number": serial }),
        )
        .await;
        ids.push(device["id"].as_i64().unwrap());
    }
    ignore(&app, &ids).await;

    for (action, command) in [("on", "On"), ("off", "Off")] {
        commands.lock().unwrap().clear();
        let (status, body) = call(&app, "POST", &format!("/api/lights/{action}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (
                &body["switched"],
                &body["no_response"],
                &body["failed"],
                &body["missing"]
            ),
            (&json!(1), &json!(1), &json!(0), &json!(1)),
            "{body}"
        );
        assert_eq!(
            results(&body, "result"),
            [
                (
                    json!(1),
                    json!(2),
                    json!("Aqara LED Bulb T2"),
                    json!("ignored")
                ),
                (json!(1), json!(3), json!("台所"), json!("switched")),
                (json!(5), json!(1), json!("読書灯"), json!("ignored")),
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
            json!([{ "id": 3, "name": "消えた灯" }])
        );
        let sent: Vec<_> = commands
            .lock()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["node_id"].clone(),
                    c["endpoint_id"].clone(),
                    c["command_name"].clone(),
                )
            })
            .collect();
        assert_eq!(sent, [(json!(1), json!(3), json!(command))]);
    }

    let (_, body) = call(&app, "GET", "/api/lights", None).await;
    assert_eq!(
        (
            &body["on"],
            &body["off"],
            &body["no_response"],
            &body["missing"]
        ),
        (&json!(0), &json!(1), &json!(1), &json!(1))
    );
    assert_eq!(body["lights"][0]["state"], "ignored");
    assert_eq!(body["lights"][2]["state"], "ignored");

    // Ignoring is undone the same way.
    let (_, _) = call(
        &app,
        "PATCH",
        "/api/devices/1",
        Some(json!({ "ignored": false })),
    )
    .await;
    let (_, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(body["switched"], 2);
}

#[tokio::test]
async fn ignored_lights_get_no_adjustment() {
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let (home, app, _) = auto_home(addr, None).await;
    enable(&app).await;
    // The Tapo, third in the ledger.
    ignore(&app, &[3]).await;

    home.adjust(NIGHT, "scheduled").await;
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["runs"][0]["lights"][2]["decision"], "ignored");
    // The Tapo is on but neither read nor written.
    assert!(!reads(&commands).contains(&(json!(5), json!(["1/6/0"]))));
    assert!(tunings(&commands).iter().all(|(node, ..)| node != 5));

    // "All on" neither switches nor adjusts it.
    commands.lock().unwrap().clear();
    let (status, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        commands
            .lock()
            .unwrap()
            .iter()
            .all(|c| c["node_id"] != 5 && c["args"]["node_id"] != 5)
    );
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(body["runs"][0]["trigger"], "lights_on");
    assert_eq!(body["runs"][0]["lights"][2]["decision"], "ignored");
}

#[test]
fn opening_a_version_4_ledger_moves_each_lights_label_to_its_labels() {
    let path = std::env::temp_dir().join(format!("home-link-v4-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let old = home_link::open_db(path.to_str().unwrap()).unwrap();
        old.execute_batch(
            "PRAGMA foreign_keys = OFF;
            DROP TABLE label_schedules;
            DROP TABLE device_labels;
            DROP TABLE devices;
            DROP TABLE labels;
            CREATE TABLE labels (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                day_level INTEGER,
                night_level INTEGER,
                cool_kelvin INTEGER,
                warm_kelvin INTEGER
            );
            CREATE TABLE rooms (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
            CREATE TABLE devices (
                id INTEGER PRIMARY KEY,
                room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
                qr_payload TEXT UNIQUE,
                vendor TEXT,
                serial_number TEXT,
                mac TEXT UNIQUE,
                name TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                label_id INTEGER REFERENCES labels(id) ON DELETE SET NULL,
                ignored INTEGER NOT NULL DEFAULT 0,
                UNIQUE (vendor, serial_number),
                CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
            );
            PRAGMA user_version = 4;
            INSERT INTO rooms (name) VALUES ('リビング');
            INSERT INTO labels (name, night_level, warm_kelvin) VALUES ('キッチン', 76, 2700);
            INSERT INTO labels (name, warm_kelvin) VALUES ('ゲームPC', 3500);
            INSERT INTO devices (id, room_id, vendor, serial_number, mac, name, label_id, ignored) VALUES
                (3, 1, 'Aqara', 'a', NULL, 'キッチン1', 1, 0), (4, 1, 'Tapo', 'b', 'AABBCCDDEEFF', 'ゲームPC1', 2, 0),
                (5, 1, 'Aqara', 'c', NULL, '消えた灯', NULL, 1);",
        )
        .unwrap();
    }
    for _ in 0..2 {
        let db = home_link::open_db(path.to_str().unwrap()).unwrap();
        assert_eq!(
            assignments(&db),
            [(3, vec![1, 3]), (4, vec![2, 3]), (5, vec![3])]
        );
        let kept: Vec<(String, Option<String>, bool)> = db
            .prepare("SELECT name, mac, ignored FROM devices ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            kept,
            [
                ("キッチン1".into(), None, false),
                ("ゲームPC1".into(), Some("AABBCCDDEEFF".into()), false),
                ("消えた灯".into(), None, true),
            ]
        );
        let has_label_id: bool = db
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM pragma_table_info('devices') WHERE name = 'label_id')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_label_id);
        // Deleting a device or a label drops its assignments.
        let fk: Vec<(String,)> = db
            .prepare(
                "SELECT \"table\" FROM pragma_foreign_key_list('device_labels') ORDER BY \"table\"",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?,)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(fk, [("devices".into(),), ("labels".into(),)]);
    }
    let db = home_link::open_db(path.to_str().unwrap()).unwrap();
    db.execute("DELETE FROM devices WHERE id = 3", []).unwrap();
    db.execute("DELETE FROM labels WHERE id IN (2, 3)", [])
        .unwrap();
    let left: i64 = db
        .query_row("SELECT count(*) FROM device_labels", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
    let _ = std::fs::remove_file(&path);
}

async fn create_label(app: &Router, body: Value) -> Value {
    let (status, label) = call(app, "POST", "/api/labels", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{label}");
    label
}

async fn assign(app: &Router, device: &Value, labels: &[&Value]) -> (StatusCode, Value) {
    let ids: Vec<_> = labels.iter().map(|l| l["id"].clone()).collect();
    call(
        app,
        "PATCH",
        &format!("/api/devices/{}", device["id"]),
        Some(json!({ "label_ids": ids })),
    )
    .await
}

#[tokio::test]
async fn a_light_takes_several_labels_of_any_traits() {
    let (app, sql) = ledger_app();
    let t2 = add_device(
        &app,
        &sql,
        json!({ "vendor": "Aqara", "serial_number": "t2-b", "name": "キッチン1" }),
    )
    .await;
    let ambient_label = create_label(&app, json!({ "name": "ambient", "auto": true })).await;
    let living = create_label(&app, json!({ "name": "living", "auto": true })).await;
    let kitchen = create_label(
        &app,
        json!({ "name": "kitchen", "override_level": 203, "override_kelvin": 5000 }),
    )
    .await;

    // The order given does not matter, nor a label given twice.
    let (status, device) = assign(&app, &t2, &[&kitchen, &ambient_label, &living, &kitchen]).await;
    assert_eq!(status, StatusCode::OK, "{device}");
    assert_eq!(
        device["labels"],
        json!([
            { "id": ambient_label["id"], "name": "ambient" },
            { "id": living["id"], "name": "living" },
            { "id": kitchen["id"], "name": "kitchen" },
        ])
    );
    let (_, labels) = call(&app, "GET", "/api/labels", None).await;
    let counts: Vec<_> = labels
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["device_count"].clone())
        .collect();
    assert_eq!(counts, [json!(1), json!(1), json!(1)]);

    // Deleting one label keeps the others.
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/labels/{}", living["id"]),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, device) = call(&app, "GET", &format!("/api/devices/{}", t2["id"]), None).await;
    assert_eq!(device["labels"].as_array().unwrap().len(), 2);
}

/// `(node_id, endpoint, command_name)` of every On/Off command.
fn switches(commands: &Arc<Mutex<Vec<Value>>>) -> Vec<(Value, Value, Value)> {
    commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c["cluster_id"] == 6)
        .map(|c| {
            (
                c["node_id"].clone(),
                c["endpoint_id"].clone(),
                c["command_name"].clone(),
            )
        })
        .collect()
}

async fn boost(app: &Router, label: &str) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        "/api/lights/boost",
        Some(json!({ "label": label })),
    )
    .await
}

fn at_once(command: &str, cluster: u64, value: u64) -> (Value, Value, Value) {
    let payload = if cluster == 8 {
        json!({ "level": value, "transitionTime": 0, "optionsMask": 1, "optionsOverride": 1 })
    } else {
        json!({ "colorTemperatureMireds": value, "transitionTime": 0, "optionsMask": 1, "optionsOverride": 1 })
    };
    (json!(cluster), json!(command), payload)
}

/// A release's tuning of a light left on: at once, but without `ExecuteIfOff`.
fn released(level: u64, mireds: u64) -> [(Value, Value, Value); 2] {
    [
        (
            json!(8),
            json!("MoveToLevel"),
            json!({ "level": level, "transitionTime": 0, "optionsMask": 0, "optionsOverride": 0 }),
        ),
        (
            json!(768),
            json!("MoveToColorTemperature"),
            json!({ "colorTemperatureMireds": mireds, "transitionTime": 0, "optionsMask": 0, "optionsOverride": 0 }),
        ),
    ]
}

/// The commands but On/Off sent to `(node, endpoint)`.
fn tunings_of(
    commands: &Arc<Mutex<Vec<Value>>>,
    node: u64,
    endpoint: u64,
) -> Vec<(Value, Value, Value)> {
    tunings(commands)
        .into_iter()
        .filter(|(n, e, ..)| (n, e) == (&json!(node), &json!(endpoint)))
        .map(|(_, _, cluster, command, payload)| (cluster, command, payload))
        .collect()
}

fn boosted(labels: &Value) -> Vec<(Value, Value)> {
    labels
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["name"].clone(), l["boosted"].clone()))
        .collect()
}

const SESAME_UUID: &str = "3DE4DE72-AAF9-25C1-8D0F-C9E019BB060C";
const SESAME_API_KEY: &str = "test-api-key-6f1d2c";
const SESAME_SECRET: &str = "2b7e151628aed2a6abf7158809cf4f3c";
const PEER: [u8; 4] = [192, 168, 1, 50];

/// One unlock as the fake CANDY HOUSE received it: path, `x-api-key` and JSON body.
type Received = Arc<Mutex<Vec<(String, String, Value)>>>;

/// A CANDY HOUSE stand-in answering every command with `status`.
async fn fake_candy_house(status: StatusCode) -> (String, Received) {
    let received = Received::default();
    let log = received.clone();
    let app = Router::new().route(
        "/api/sesame2/{uuid}/cmd",
        axum::routing::post(
            move |uri: axum::http::Uri, headers: axum::http::HeaderMap, body: String| async move {
                let key = headers["x-api-key"].to_str().unwrap().to_owned();
                let body = serde_json::from_str(&body).unwrap();
                log.lock().unwrap().push((uri.path().to_owned(), key, body));
                status
            },
        ),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), received)
}

fn door_app(sesame: Result<home_link::sesame::Config, &'static str>) -> Router {
    home_link::Home::new(home_link::open_db(":memory:").unwrap(), None)
        .with_clock(|| 1_791_856_659)
        .with_sesame(sesame)
        .router()
}

fn sesame_config(api_url: &str) -> Result<home_link::sesame::Config, &'static str> {
    home_link::sesame::Config::new(
        Some(SESAME_API_KEY),
        Some(SESAME_UUID),
        Some(SESAME_SECRET),
        api_url,
    )
}

/// The door commands: the API path's last segment and the CANDY HOUSE command code.
const DOOR: [(&str, u64); 2] = [("unlock", 83), ("lock", 82)];

/// Sends `POST /api/door/{action}` as if from `ip`.
async fn door_from(
    app: &Router,
    action: &str,
    ip: impl Into<std::net::IpAddr>,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/api/door/{action}"))
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(SocketAddr::new(
            ip.into(),
            50_000,
        )));
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Collects every log line, at every level, written while the guard lives.
fn capture_logs() -> (Arc<Mutex<Vec<u8>>>, tracing::subscriber::DefaultGuard) {
    #[derive(Clone)]
    struct Logs(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Logs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // While one scoped dispatcher is alive, tracing-core asks only the registering
    // thread's, so a callsite first hit on another test's thread would never log;
    // a second one kept alive makes it ask every live dispatcher.
    static SECOND: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    SECOND.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer = Logs(logs.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    (logs, tracing::subscriber::set_default(subscriber))
}

fn assert_no_keys(text: &str) {
    assert!(!text.contains(SESAME_API_KEY), "API key leaked: {text}");
    assert!(
        !text.to_lowercase().contains(&SESAME_UUID.to_lowercase()),
        "UUID leaked: {text}"
    );
    assert!(
        !text.to_lowercase().contains(SESAME_SECRET),
        "secret key leaked: {text}"
    );
}

#[tokio::test]
async fn door_commands_are_signed_and_sent_to_candy_house() {
    let (logs, _guard) = capture_logs();
    for (action, cmd) in DOOR {
        let (api_url, received) = fake_candy_house(StatusCode::OK).await;
        let app = door_app(sesame_config(&api_url));

        let (status, body) = door_from(&app, action, PEER).await;
        assert_eq!(status, StatusCode::OK, "{action}: {body}");
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!({ "status": "accepted" })
        );
        assert_no_keys(&body);
        let (status, _) =
            door_from(&app, action, "fd00::5".parse::<std::net::IpAddr>().unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{action}");

        let received = received.lock().unwrap().clone();
        assert_eq!(received.len(), 2, "{action}");
        let (path, key, command) = &received[0];
        assert_eq!(path, &format!("/api/sesame2/{SESAME_UUID}/cmd"));
        assert_eq!(key, SESAME_API_KEY);
        // The signature for 1791856659 (bytes 90 cd 6a), computed with `openssl mac ... CMAC`.
        assert_eq!(
            command,
            &json!({ "cmd": cmd, "history": "aG9tZS1saW5r", "sign": "48751c14a4d146ec34bfd1d580c74164" }),
            "{action}"
        );
    }
    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    for accepted in ["unlock accepted", "lock accepted"] {
        assert!(logs.contains(accepted), "logs were not captured: {logs}");
    }
    assert_no_keys(&logs);
}

#[tokio::test]
async fn door_commands_report_refusals_and_lost_connections_as_failures() {
    let (logs, _guard) = capture_logs();
    let (refusing, _) = fake_candy_house(StatusCode::UNAUTHORIZED).await;

    // Accepts the connection, then drops it without answering.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dropping = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            drop(socket);
        }
    });
    // Nothing listens there any more.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_addr = closed.local_addr().unwrap();
    drop(closed);

    for (action, _) in DOOR {
        let (status, body) = door_from(&door_app(sesame_config(&refusing)), action, PEER).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{action}: {body}");
        let error: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(error["error"], "sesame_rejected");
        assert!(error["message"].as_str().unwrap().contains("401"), "{body}");
        assert_no_keys(&body);

        for addr in [dropping, closed_addr] {
            let app = door_app(sesame_config(&format!("http://{addr}")));
            let (status, body) = door_from(&app, action, PEER).await;
            assert_eq!(status, StatusCode::BAD_GATEWAY, "{action}: {body}");
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap()["error"],
                "sesame_unreachable"
            );
            assert_no_keys(&body);
        }
    }

    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(
        logs.contains("CANDY HOUSE unreachable"),
        "logs were not captured: {logs}"
    );
    assert_no_keys(&logs);
}

#[tokio::test]
async fn door_commands_without_settings_say_so_and_leave_other_apis_working() {
    let app = door_app(home_link::sesame::Config::new(
        None,
        None,
        None,
        "http://unused",
    ));
    for (action, _) in DOOR {
        let (status, body) = door_from(&app, action, PEER).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{action}: {body}");
        let error: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(error["error"], "sesame_not_configured");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("SESAME_API_KEY"),
            "{body}"
        );
        // The plain app, as built from the environment, has no SESAME either.
        let (status, _) = door_from(&app_unset(), action, PEER).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{action}");
    }

    let (status, _) = call(&app, "GET", "/api/labels", None).await;
    assert_eq!(status, StatusCode::OK);
}

fn app_unset() -> Router {
    home_link::app(home_link::open_db(":memory:").unwrap(), None)
}

#[tokio::test]
async fn door_commands_reach_candy_house_from_any_address() {
    for (action, cmd) in DOOR {
        let (api_url, received) = fake_candy_house(StatusCode::OK).await;
        let app = door_app(sesame_config(&api_url));
        let ips = [
            "192.168.1.50",
            "127.0.0.1",
            "172.17.0.1",
            "100.128.0.1",
            "::1",
            "::ffff:192.168.1.50",
        ];
        for ip in ips {
            let (status, body) =
                door_from(&app, action, ip.parse::<std::net::IpAddr>().unwrap()).await;
            assert_eq!(status, StatusCode::OK, "{action} {ip}: {body}");
        }
        let received = received.lock().unwrap().clone();
        assert_eq!(received.len(), ips.len(), "{action}");
        assert!(received.iter().all(|(_, _, command)| command["cmd"] == cmd));
    }
}

/// How the fake bulb takes a move to another Wi-Fi.
#[derive(Clone, Copy, PartialEq)]
enum Join {
    /// Answers `ConnectNetwork` and completes on the new network.
    Answers,
    /// Leaves without answering `ConnectNetwork`; the first completion fails, the next lands,
    /// and matterjs-server cannot read it again yet.
    Silent,
    /// Refuses the new network in its `ConnectNetwork` answer, then answers on the old one.
    Refuses,
    /// Never comes up on the new network; after the fail-safe it answers on the old one.
    Reverts,
    /// Never answers again.
    Lost,
}

const HOME_2G: &str = "aG9tZS0yZw==";
const IOT_2G: &str = "aW90LTJn";
const IOT_PASSWORD_BASE64: &str = "aW90LWhpbWl0c3UtOTFjMg==";

/// A Tapo bulb on Wi-Fi "home-2g", the only network it holds.
fn wifi_bulb() -> Value {
    json!({ "node_id": 17, "available": true, "attributes": {
        "0/40/1": "Tapo", "0/40/15": TAPO, "0/40/3": "Smart Bulb",
        "0/49/65532": 1, "0/49/0": 1, "0/49/1": [{ "0": HOME_2G, "1": true }],
    }})
}

/// Serves `nodes` and answers a Wi-Fi move the way `join` says, recording every command.
/// Like matterjs-server, the nodes served keep the old network until the bulb is interviewed.
async fn fake_mover(mut nodes: Value, join: Join) -> (SocketAddr, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let seen = commands.clone();
    tokio::spawn(async move {
        let completions = AtomicUsize::new(0);
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::text(server_info(true, false).to_string()))
                .await
                .unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let command: Value = serde_json::from_str(&text).unwrap();
                seen.lock().unwrap().push(command.clone());
                let id = command["message_id"].clone();
                let ok = |result: Value| json!({ "message_id": id, "result": result });
                let unreachable = json!({ "message_id": id, "error_code": 0, "details": "Node 17 is not connected" });
                let networking =
                    |status: u64| ok(json!({ "networkingStatus": status, "networkIndex": 0 }));
                let reply = match command["command"].as_str().unwrap() {
                    "get_nodes" => ok(nodes.clone()),
                    "remove_wifi_credentials" => ok(json!({})),
                    "interview_node" if join == Join::Silent => unreachable,
                    "interview_node" => {
                        nodes[0]["attributes"]["0/49/1"] = json!([{ "0": IOT_2G, "1": true }]);
                        ok(Value::Null)
                    }
                    "read_attribute" if join == Join::Lost => unreachable,
                    "read_attribute" => ok(json!({ "0/49/1": [{ "0": HOME_2G, "1": true }] })),
                    "device_command" => match command["args"]["command_name"].as_str().unwrap() {
                        "ArmFailSafe" => ok(json!({ "errorCode": 0, "debugText": "" })),
                        "RemoveNetwork" | "AddOrUpdateWiFiNetwork" => networking(0),
                        "ConnectNetwork" => match join {
                            Join::Answers => networking(0),
                            // AuthFailure.
                            Join::Refuses => networking(7),
                            _ => continue,
                        },
                        "CommissioningComplete" => {
                            let attempt = completions.fetch_add(1, Ordering::Relaxed);
                            match join {
                                Join::Answers => ok(json!({ "errorCode": 0, "debugText": "" })),
                                Join::Silent if attempt > 0 => {
                                    ok(json!({ "errorCode": 0, "debugText": "" }))
                                }
                                _ => unreachable,
                            }
                        }
                        other => panic!("unexpected device command {other}"),
                    },
                    other => panic!("unexpected command {other}"),
                };
                ws.send(Message::text(reply.to_string())).await.unwrap();
            }
        }
    });
    (addr, commands)
}

/// A ledger with the bulb and "home-2g" and "iot-2g" saved, "iot-2g" for devices.
async fn moving_app(addr: SocketAddr) -> (Router, rusqlite::Connection, Value) {
    let (db, sql) = ledger();
    sql.execute(
        "INSERT INTO wifi_networks (ssid, password, for_devices) VALUES ('home-2g', ?1, 0), ('iot-2g', ?2, 1)",
        [PASSWORD, IOT_PASSWORD],
    )
    .unwrap();
    let app = home_link::Home::new(db, Some(format!("ws://{addr}/ws")))
        .with_wifi_move_times(home_link::commission::MoveTimes {
            fail_safe: std::time::Duration::from_secs(1),
            connect: std::time::Duration::from_millis(200),
            retry: std::time::Duration::from_millis(50),
            settle: std::time::Duration::from_millis(300),
        })
        .router();
    let device = add_device(
        &app,
        &sql,
        json!({ "qr_payload": QR, "vendor": "Tapo", "serial_number": TAPO, "mac": TAPO, "name": "押入れ1" }),
    )
    .await;
    (app, sql, device)
}

async fn move_wifi(app: &Router, device: &Value) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        &format!("/api/devices/{}/wifi", device["id"]),
        None,
    )
    .await
}

/// The device commands sent, by name, with the other commands but reading nodes.
fn moves(commands: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    sent(commands)
        .iter()
        .map(|c| match c["command"].as_str().unwrap() {
            "device_command" => c["args"]["command_name"].clone(),
            other => json!(other),
        })
        .collect()
}

/// No table but the saved networks holds `secret`.
fn assert_only_saved(sql: &rusqlite::Connection, secret: &str) {
    let tables: Vec<String> = sql
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name != 'wifi_networks'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for table in tables {
        let rows: Vec<String> = sql
            .prepare(&format!(
                "SELECT quote(t.*) FROM (SELECT * FROM \"{table}\") AS t"
            ))
            .and_then(|mut s| {
                s.query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap_or_default();
        assert!(
            rows.iter().all(|row| !row.contains(secret)),
            "{table} holds the password"
        );
    }
}

#[tokio::test]
async fn devices_show_the_wifi_they_are_on() {
    let (addr, _) = fake_mover(json!([wifi_bulb()]), Join::Answers).await;
    let (app, _, device) = moving_app(addr).await;
    assert_eq!(device["wifi_ssid"], "home-2g");
    let (_, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(list[0]["wifi_ssid"], "home-2g");
}

#[tokio::test]
async fn a_bulb_moves_to_the_wifi_for_devices_under_a_fail_safe() {
    let (logs, _guard) = capture_logs();
    let (addr, commands) = fake_mover(json!([wifi_bulb()]), Join::Answers).await;
    let (app, sql, device) = moving_app(addr).await;
    let (status, body) = move_wifi(&app, &device).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["device"]["wifi_ssid"], "iot-2g");
    assert_eq!(
        moves(&commands),
        [
            "ArmFailSafe",
            "RemoveNetwork",
            "AddOrUpdateWiFiNetwork",
            "ConnectNetwork",
            "CommissioningComplete",
            "interview_node",
            "remove_wifi_credentials"
        ]
    );
    assert_eq!(sent(&commands)[5]["args"], json!({ "node_id": 17 }));
    let args: Vec<Value> = sent(&commands).iter().map(|c| c["args"].clone()).collect();
    for (args, cluster) in args.iter().zip([48, 49, 49, 49, 48]) {
        assert_eq!(
            (&args["node_id"], &args["endpoint_id"], &args["cluster_id"]),
            (&json!(17), &json!(0), &json!(cluster))
        );
    }
    assert_eq!(
        args[0]["payload"],
        json!({ "expiryLengthSeconds": 1, "breadcrumb": 0 })
    );
    assert_eq!(
        args[1]["payload"],
        json!({ "networkId": HOME_2G, "breadcrumb": 0 })
    );
    assert_eq!(
        args[2]["payload"],
        json!({ "ssid": IOT_2G, "credentials": IOT_PASSWORD_BASE64, "breadcrumb": 0 })
    );
    assert_eq!(
        args[3]["payload"],
        json!({ "networkId": IOT_2G, "breadcrumb": 0 })
    );
    assert_eq!(args[4]["payload"], json!({}));

    let (_, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(list[0]["wifi_ssid"], "iot-2g");
    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("wifi"), "logs were not captured: {logs}");
    for secret in [IOT_PASSWORD, IOT_PASSWORD_BASE64] {
        assert!(!body.to_string().contains(secret) && !list.to_string().contains(secret));
        assert!(!logs.contains(secret), "password in logs: {logs}");
        assert_only_saved(&sql, secret);
    }
}

#[tokio::test]
async fn a_bulb_that_drops_off_without_answering_still_moves() {
    let (addr, commands) = fake_mover(json!([wifi_bulb()]), Join::Silent).await;
    let (app, _, device) = moving_app(addr).await;
    let (status, body) = move_wifi(&app, &device).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["device"]["wifi_ssid"], "iot-2g");
    assert_eq!(
        moves(&commands),
        [
            "ArmFailSafe",
            "RemoveNetwork",
            "AddOrUpdateWiFiNetwork",
            "ConnectNetwork",
            "CommissioningComplete",
            "CommissioningComplete",
            // Failing to read the bulb again leaves the move done.
            "interview_node",
            "remove_wifi_credentials"
        ]
    );
}

#[tokio::test]
async fn a_bulb_that_cannot_join_goes_back_to_its_wifi_and_says_so() {
    for join in [Join::Refuses, Join::Reverts] {
        let (addr, commands) = fake_mover(json!([wifi_bulb()]), join).await;
        let (app, _, device) = moving_app(addr).await;
        let started = std::time::Instant::now();
        let (status, body) = move_wifi(&app, &device).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("wifi_reverted")),
            "{body}"
        );
        assert!(
            body["message"].as_str().unwrap().contains("iot-2g"),
            "{body}"
        );
        let moves = moves(&commands);
        assert_eq!(
            moves[..4],
            [
                "ArmFailSafe",
                "RemoveNetwork",
                "AddOrUpdateWiFiNetwork",
                "ConnectNetwork"
            ]
        );
        // The fail-safe is dropped at once, which reverts the network, and the bulb read back.
        let tail = &moves[moves.len() - 3..];
        assert_eq!(
            tail,
            ["ArmFailSafe", "read_attribute", "remove_wifi_credentials"]
        );
        let disarm = sent(&commands).into_iter().rev().nth(2).unwrap();
        assert_eq!(
            disarm["args"]["payload"],
            json!({ "expiryLengthSeconds": 0, "breadcrumb": 0 })
        );
        if join == Join::Refuses {
            assert!(!moves.contains(&json!("CommissioningComplete")));
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
        }
    }
}

#[tokio::test]
async fn a_bulb_that_answers_nothing_is_said_to_be_going_back() {
    let (addr, commands) = fake_mover(json!([wifi_bulb()]), Join::Lost).await;
    let (app, _, device) = moving_app(addr).await;
    let (status, body) = move_wifi(&app, &device).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::GATEWAY_TIMEOUT, Some("wifi_reverting")),
        "{body}"
    );
    assert_eq!(moves(&commands).last().unwrap(), "remove_wifi_credentials");
}

#[tokio::test]
async fn moving_needs_a_wifi_bulb_off_the_wifi_for_devices() {
    // Already on the Wi-Fi for devices: nothing to send.
    let mut on_iot = wifi_bulb();
    on_iot["attributes"]["0/49/1"] = json!([{ "0": IOT_2G, "1": true }]);
    let (addr, commands) = fake_mover(json!([on_iot]), Join::Lost).await;
    let (app, sql, device) = moving_app(addr).await;
    let (status, body) = move_wifi(&app, &device).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["device"]["wifi_ssid"], "iot-2g");
    assert_eq!(moves(&commands), Vec::<Value>::new());

    // Not on Wi-Fi: a Thread bulb, or one matterjs-server does not serve.
    let mut on_thread = wifi_bulb();
    on_thread["attributes"]["0/49/65532"] = json!(2);
    on_thread["attributes"]["0/49/1"] = json!([{ "0": "AAECAwQFBgc=", "1": true }]);
    for nodes in [json!([on_thread]), json!([])] {
        let (addr, commands) = fake_mover(nodes, Join::Lost).await;
        let (app, _, device) = moving_app(addr).await;
        let (status, body) = move_wifi(&app, &device).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::CONFLICT, Some("not_on_wifi")),
            "{body}"
        );
        assert_eq!(moves(&commands), Vec::<Value>::new());
    }

    // No Wi-Fi for devices, and no such device.
    sql.execute("UPDATE wifi_networks SET for_devices = 0", [])
        .unwrap();
    let (status, body) = move_wifi(&app, &device).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("wifi_not_set"))
    );
    let (status, body) = move_wifi(&app, &json!({ "id": 999 })).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::NOT_FOUND, Some("device_not_found"))
    );
}

/// A Wi-Fi bulb, an On/Off and a Dimmable Plug-in Unit, a silent and an
/// unavailable plug, a plug behind a bridge and a sensor.
fn plug_nodes() -> Value {
    json!([
        { "node_id": 5, "available": true, "attributes": {
            "0/40/1": "Tapo", "0/40/15": TAPO, "0/40/3": "Smart Multicolor Bulb",
            "0/29/0": [{ "0": 22, "1": 1 }],
            "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": true, "1/8/0": 100,
            "1/768/65532": 16,
        }},
        { "node_id": 20, "available": true, "attributes": {
            "0/40/1": "Plugco", "0/40/15": "plug-a", "0/40/3": "Mini Plug",
            "0/29/0": [{ "0": 22, "1": 1 }],
            "1/29/0": [{ "0": 266, "1": 1 }], "1/6/0": false,
        }},
        { "node_id": 21, "available": true, "attributes": {
            "0/40/1": "Plugco", "0/40/15": "plug-b", "0/40/3": "Dimmer Plug",
            "1/29/0": [{ "0": 267, "1": 1 }], "1/6/0": true, "1/8/0": 100,
        }},
        { "node_id": 22, "available": true, "attributes": {
            "0/40/1": "Plugco", "0/40/15": "plug-c",
            "1/29/0": [{ "0": 266, "1": 1 }], "1/6/0": true,
        }},
        { "node_id": 23, "available": false, "attributes": {
            "0/40/1": "Plugco", "0/40/15": "plug-d",
            "1/29/0": [{ "0": 266, "1": 1 }], "1/6/0": true,
        }},
        { "node_id": 30, "available": true, "attributes": {
            "0/40/1": "Hubco", "0/40/15": "hub", "1/29/0": [{ "0": 14, "1": 1 }],
            "2/29/0": [{ "0": 19, "1": 2 }, { "0": 266, "1": 1 }], "2/6/0": false,
            "2/57/1": "Hubco", "2/57/15": "plug-e", "2/57/3": "Bridged Plug", "2/57/17": true,
            "3/29/0": [{ "0": 19, "1": 2 }, { "0": 770, "1": 1 }],
            "3/57/1": "Hubco", "3/57/15": "sensor", "3/57/17": true,
        }},
    ])
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn plugs_get_their_kind_and_are_switched_one_at_a_time() {
    let fake = Fake {
        silent: vec![(22, 1)],
        ..Fake::default()
    };
    let (addr, commands, _) = fake_light_server(plug_nodes(), fake).await;
    let (db, sql) = ledger();
    let home = home_link::Home::new(db, Some(format!("ws://{addr}/ws")));
    let app = home.router();
    let mut ids = Vec::new();
    for (vendor, serial) in [
        ("Tapo", TAPO),
        ("Plugco", "plug-a"),
        ("Plugco", "plug-b"),
        ("Plugco", "plug-c"),
        ("Plugco", "plug-d"),
        ("Hubco", "plug-e"),
        ("Hubco", "sensor"),
        ("Hubco", "hub"),
    ] {
        let device = add_device(
            &app,
            &sql,
            json!({ "vendor": vendor, "serial_number": serial }),
        )
        .await;
        ids.push(device["id"].clone());
    }
    let unconnected = add_device(&app, &sql, json!({ "manual_code": MANUAL })).await;

    let (status, list) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let kinds: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["kind"].clone(), d["on"].clone()))
        .collect();
    assert_eq!(
        kinds,
        [
            (json!("light"), Value::Null),
            (json!("plug"), json!(false)),
            (json!("plug"), json!(true)),
            (json!("plug"), json!(true)),
            (json!("plug"), json!(true)),
            (json!("plug"), json!(false)),
            (json!("other"), Value::Null),
            (json!("other"), Value::Null),
            (Value::Null, Value::Null),
        ]
    );
    assert_eq!(unconnected["kind"], Value::Null);

    let switch = |id: &Value, action: &str| format!("/api/devices/{id}/{action}");
    // Each plug's own On/Off, behind a bridge on its endpoint.
    for (id, action, node, endpoint, command, state) in [
        (&ids[1], "on", 20, 1, "On", "on"),
        (&ids[1], "off", 20, 1, "Off", "off"),
        (&ids[2], "off", 21, 1, "Off", "off"),
        (&ids[5], "on", 30, 2, "On", "on"),
    ] {
        commands.lock().unwrap().clear();
        let (status, body) = call(&app, "POST", &switch(id, action), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["state"], state);
        assert_eq!(
            sent(&commands),
            [
                json!({ "node_id": node, "endpoint_id": endpoint, "cluster_id": 6,
                     "command_name": command, "payload": {} })
            ]
        );
    }
    let (_, device) = call(&app, "GET", &format!("/api/devices/{}", ids[1]), None).await;
    assert_eq!(device["on"], false);
    let (_, device) = call(&app, "GET", &format!("/api/devices/{}", ids[5]), None).await;
    assert_eq!(device["on"], true);

    // No answer, or unavailable and so not sent to.
    commands.lock().unwrap().clear();
    let (status, body) = call(&app, "POST", &switch(&ids[3], "off"), None).await;
    assert_eq!(
        (status, &body["state"]),
        (StatusCode::OK, &json!("no_response"))
    );
    assert_eq!(sent(&commands).len(), 1);
    commands.lock().unwrap().clear();
    let (status, body) = call(&app, "POST", &switch(&ids[4], "off"), None).await;
    assert_eq!(
        (status, &body["state"]),
        (StatusCode::OK, &json!("no_response"))
    );
    assert_eq!(sent(&commands), Vec::<Value>::new());

    // A light, a device without On/Off and one not connected are not switched here.
    for (id, error) in [
        (&ids[0], "not_switchable"),
        (&ids[6], "not_switchable"),
        (&unconnected["id"], "not_connected"),
    ] {
        let (status, body) = call(&app, "POST", &switch(id, "on"), None).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::CONFLICT, Some(error)),
            "{body}"
        );
    }
    let (status, _) = call(&app, "POST", "/api/devices/999/on", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(sent(&commands), Vec::<Value>::new());

    // All on, all off and the adjustment leave every plug alone.
    enable(&app).await;
    for action in ["on", "off"] {
        let (status, body) = call(&app, "POST", &format!("/api/lights/{action}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    home.adjust(NIGHT, "scheduled").await;
    let touched: Vec<_> = commands
        .lock()
        .unwrap()
        .iter()
        .map(|c| {
            c["node_id"]
                .as_u64()
                .or(c["args"]["node_id"].as_u64())
                .unwrap()
        })
        .collect();
    assert!(!touched.is_empty());
    assert!(touched.iter().all(|node| *node == 5), "{touched:?}");
}

async fn label_op(app: &Router, label: &Value, op: &str) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        &format!("/api/labels/{}/{op}", label["id"]),
        None,
    )
    .await
}

fn device_results(body: &Value) -> Vec<(Value, Value, Value)> {
    body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["node_id"].clone(),
                d["endpoint"].clone(),
                d["result"].clone(),
            )
        })
        .collect()
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn a_labels_on_off_and_toggle_reach_its_lights_and_plugs() {
    let (addr, commands, _) = fake_light_server(plug_nodes(), Fake::default()).await;
    let (db, sql) = ledger();
    let app = home_link::app(db, Some(format!("ws://{addr}/ws")));
    let mut devices = Vec::new();
    for (vendor, serial) in [
        ("Tapo", TAPO),
        ("Plugco", "plug-a"),
        ("Plugco", "plug-b"),
        ("Hubco", "plug-e"),
        ("Hubco", "sensor"),
        ("Plugco", "gone"),
    ] {
        devices.push(
            add_device(
                &app,
                &sql,
                json!({ "vendor": vendor, "serial_number": serial }),
            )
            .await,
        );
    }
    let mixed = create_label(&app, json!({ "name": "mixed" })).await;
    let razor = create_label(&app, json!({ "name": "razor" })).await;
    for device in [&devices[0], &devices[3], &devices[4], &devices[5]] {
        assert_eq!(assign(&app, device, &[&mixed]).await.0, StatusCode::OK);
    }
    assert_eq!(assign(&app, &devices[1], &[&razor]).await.0, StatusCode::OK);

    // The light and the bridged plug; the sensor has no On/Off, the other plugs are not under it.
    let (status, body) = label_op(&app, &mixed, "on").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            &body["label"],
            &body["action"],
            &body["on"],
            &body["boosted"]
        ),
        (&json!("mixed"), &json!("on"), &json!(true), &json!(false))
    );
    assert_eq!(
        (
            &body["switched"],
            &body["no_response"],
            &body["failed"],
            &body["missing"]
        ),
        (&json!(2), &json!(0), &json!(0), &json!(1))
    );
    assert_eq!(
        device_results(&body),
        [
            (json!(5), json!(1), json!("switched")),
            (json!(30), json!(2), json!("switched")),
        ]
    );
    assert_eq!(body["missing_devices"][0]["id"], devices[5]["id"]);
    assert_eq!(
        switches(&commands),
        [
            (json!(5), json!(1), json!("On")),
            (json!(30), json!(2), json!("On"))
        ]
    );

    commands.lock().unwrap().clear();
    let (_, body) = label_op(&app, &mixed, "off").await;
    assert_eq!(
        (&body["action"], &body["on"]),
        (&json!("off"), &json!(false))
    );
    assert_eq!(
        switches(&commands),
        [
            (json!(5), json!(1), json!("Off")),
            (json!(30), json!(2), json!("Off"))
        ]
    );

    // A toggle switches to the other state.
    for (action, command) in [("on", "On"), ("off", "Off")] {
        commands.lock().unwrap().clear();
        let (status, body) = label_op(&app, &mixed, "toggle").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["action"], action);
        assert_eq!(
            switches(&commands),
            [
                (json!(5), json!(1), json!(command)),
                (json!(30), json!(2), json!(command))
            ]
        );
    }
    commands.lock().unwrap().clear();
    let (_, body) = label_op(&app, &razor, "toggle").await;
    assert_eq!(
        (&body["action"], &body["switched"]),
        (&json!("on"), &json!(1))
    );
    assert_eq!(switches(&commands), [(json!(20), json!(1), json!("On"))]);
    let (_, labels) = call(&app, "GET", "/api/labels", None).await;
    let label_states: Vec<_> = labels
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["on"].clone())
        .collect();
    assert_eq!(label_states, [json!(false), json!(true)]);

    // Unknown labels, labels without devices and with only ignored ones.
    commands.lock().unwrap().clear();
    let (status, error) = call(&app, "POST", "/api/labels/999/on", None).await;
    assert_eq!(
        (status, &error["error"]),
        (StatusCode::NOT_FOUND, &json!("label_not_found"))
    );
    let empty = create_label(&app, json!({ "name": "empty" })).await;
    for op in ["on", "off", "toggle"] {
        let (status, error) = label_op(&app, &empty, op).await;
        assert_eq!(
            (status, &error["error"]),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                &json!("label_has_no_devices")
            )
        );
    }
    ignore(&app, &[devices[1]["id"].as_i64().unwrap()]).await;
    let (status, _) = label_op(&app, &razor, "off").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        commands
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c["cluster_id"] == 6)
            .count(),
        0
    );
}

#[tokio::test]
async fn only_the_lights_of_an_auto_label_are_adjusted() {
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let (db, sql) = ledger();
    let home = home_link::Home::new(db, Some(format!("ws://{addr}/ws")));
    let app = home.router();
    enable(&app).await;
    let ambient = create_label(&app, json!({ "name": "ambient", "auto": true })).await;
    let manual = create_label(&app, json!({ "name": "hallway" })).await;
    let game = create_label(
        &app,
        json!({ "name": "game", "override_level": 203, "override_kelvin": 5000 }),
    )
    .await;
    let mut devices = Vec::new();
    for (vendor, serial) in TUNING_LIGHTS {
        devices.push(
            add_device(
                &app,
                &sql,
                json!({ "vendor": vendor, "serial_number": serial }),
            )
            .await,
        );
    }
    // The T2 that is on under ambient and game; the Tapo that is on under the label
    // without auto; the plain light unlabelled.
    assert_eq!(
        assign(&app, &devices[1], &[&ambient, &game]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        assign(&app, &devices[2], &[&manual]).await.0,
        StatusCode::OK
    );

    home.adjust(NIGHT, "scheduled").await;
    assert_eq!(reads(&commands), [(json!(1), json!(["3/6/0"]))]);
    assert_eq!(
        sent_values(&commands),
        [
            (json!(1), json!(3), json!(102)),
            (json!(1), json!(3), json!(333))
        ]
    );
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(
        decisions(&body["runs"][0]),
        [
            (json!(1), json!(2), json!("manual")),
            (json!(1), json!(3), json!("sent")),
            (json!(5), json!(1), json!("manual")),
            (json!(6), json!(1), json!("manual")),
            (json!(16), json!(1), json!("manual")),
            (json!(30), json!(1), json!("manual")),
        ]
    );
    assert_eq!(
        body["runs"][0]["lights"][1]["labels"],
        json!(["ambient", "game"])
    );
}

/// A home over `tuning_nodes` at `clock`: the T2 (1/3, on) under the auto "ambient" and
/// "game" (80%, 5000 K), the first Tapo (5/1, on) under "game" alone and the desk Tapo
/// (6/1, off) under "work" (100%, 5000 K).
async fn override_home(
    addr: SocketAddr,
    clock: fn() -> i64,
) -> (home_link::Home, Router, [Value; 3]) {
    let (db, sql) = ledger();
    let home = home_link::Home::new(db, Some(format!("ws://{addr}/ws"))).with_clock(clock);
    let app = home.router();
    enable(&app).await;
    let ambient = create_label(&app, json!({ "name": "ambient", "auto": true })).await;
    let game = create_label(
        &app,
        json!({ "name": "game", "override_level": 203, "override_kelvin": 5000 }),
    )
    .await;
    let work = create_label(
        &app,
        json!({ "name": "work", "override_level": 254, "override_kelvin": 5000 }),
    )
    .await;
    for (serial, labels) in [
        ("t2-b", vec![&ambient, &game]),
        (TAPO, vec![&game]),
        ("tapo-desk", vec![&work]),
    ] {
        let vendor = if serial == "t2-b" { "Aqara" } else { "Tapo" };
        let device = add_device(
            &app,
            &sql,
            json!({ "vendor": vendor, "serial_number": serial }),
        )
        .await;
        assert_eq!(assign(&app, &device, &labels).await.0, StatusCode::OK);
    }
    (home, app, [ambient, game, work])
}

#[tokio::test]
async fn an_override_label_lights_at_its_values_and_is_not_adjusted() {
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let (home, app, [_, _, work]) = override_home(addr, || AFTER_MIDNIGHT).await;

    let (status, body) = label_op(&app, &work, "on").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            &body["action"],
            &body["on"],
            &body["boosted"],
            &body["switched"]
        ),
        (&json!("on"), &json!(true), &json!(true), &json!(1))
    );
    assert_eq!(switches(&commands), [(json!(6), json!(1), json!("On"))]);
    assert_eq!(
        tunings_of(&commands, 6, 1),
        [
            at_once("MoveToLevelWithOnOff", 8, 254),
            at_once("MoveToColorTemperature", 768, 200)
        ]
    );

    // The schedule leaves it at 100% after midnight; "all on" leaves it to its label.
    commands.lock().unwrap().clear();
    home.adjust(AFTER_MIDNIGHT, "scheduled").await;
    assert_eq!(tunings_of(&commands, 6, 1), []);
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(decisions(&body["runs"][0])[3].2, "override");
    let (_, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert!(!switches(&commands).contains(&(json!(6), json!(1), json!("On"))));
    assert_eq!(body["failed"], 0);
    assert_eq!(body["lights"][3]["result"], "override");

    // Pressed again it stops overriding; under no other label, the light goes off.
    commands.lock().unwrap().clear();
    let (_, body) = label_op(&app, &work, "toggle").await;
    assert_eq!(
        (
            &body["action"],
            &body["on"],
            &body["boosted"],
            &body["switched"]
        ),
        (&json!("release"), &json!(false), &json!(false), &json!(1))
    );
    assert_eq!(switches(&commands), [(json!(6), json!(1), json!("Off"))]);
    assert_eq!(tunings_of(&commands, 6, 1), []);

    // Its off goes off too, and "all on" still leaves it.
    label_op(&app, &work, "toggle").await;
    commands.lock().unwrap().clear();
    let (_, body) = label_op(&app, &work, "off").await;
    assert_eq!(
        (&body["action"], &body["boosted"]),
        (&json!("off"), &json!(false))
    );
    assert_eq!(switches(&commands), [(json!(6), json!(1), json!("Off"))]);
    let (_, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(body["lights"][3]["result"], "override");
}

#[tokio::test]
async fn a_released_override_returns_each_light_to_what_its_other_labels_say() {
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let (home, app, [ambient, game, _]) = override_home(addr, || AFTER_MIDNIGHT).await;

    let (_, body) = label_op(&app, &game, "toggle").await;
    assert_eq!(
        (&body["action"], &body["boosted"]),
        (&json!("on"), &json!(true))
    );
    for (node, endpoint) in [(1, 3), (5, 1)] {
        assert_eq!(
            tunings_of(&commands, node, endpoint),
            [
                at_once("MoveToLevelWithOnOff", 8, 203),
                at_once("MoveToColorTemperature", 768, 200)
            ]
        );
    }
    commands.lock().unwrap().clear();
    home.adjust(AFTER_MIDNIGHT, "scheduled").await;
    assert_eq!(tunings(&commands), []);

    // The T2 is under ambient, which is on: back at once to half the night's level.
    // The Tapo is under no other label: off.
    let (_, body) = label_op(&app, &game, "toggle").await;
    assert_eq!(
        (&body["action"], &body["boosted"], &body["switched"]),
        (&json!("release"), &json!(false), &json!(2))
    );
    assert_eq!(switches(&commands), [(json!(5), json!(1), json!("Off"))]);
    assert_eq!(tunings_of(&commands, 1, 3), released(51, 333));

    // Room's off ends game's override of the T2 too.
    label_op(&app, &game, "on").await;
    commands.lock().unwrap().clear();
    let (_, body) = label_op(&app, &ambient, "off").await;
    assert_eq!(body["switched"], 1);
    assert_eq!(switches(&commands), [(json!(1), json!(3), json!("Off"))]);
    let (_, labels) = call(&app, "GET", "/api/labels", None).await;
    assert_eq!(boosted(&labels)[1], (json!("game"), json!(false)));

    // With ambient off, a released T2 goes off.
    label_op(&app, &game, "on").await;
    commands.lock().unwrap().clear();
    label_op(&app, &game, "toggle").await;
    let off: Vec<_> = switches(&commands)
        .into_iter()
        .map(|s| (s.0, s.2))
        .collect();
    assert_eq!(off, [(json!(1), json!("Off")), (json!(5), json!("Off"))]);
}

#[tokio::test]
async fn an_override_that_reaches_no_light_is_not_kept() {
    let fake = Fake {
        silent: vec![(6, 1)],
        ..Fake::default()
    };
    let (addr, _, _) = fake_light_server(tuning_nodes(), fake).await;
    let (_, app, [_, _, work]) = override_home(addr, || NIGHT).await;
    let (status, body) = label_op(&app, &work, "on").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            &body["on"],
            &body["boosted"],
            &body["switched"],
            &body["no_response"]
        ),
        (&Value::Null, &json!(false), &json!(0), &json!(1))
    );
}

/// 2026-10-06 00:00 in Tokyo.
const MIDNIGHT: i64 = NIGHT + 3600;

#[tokio::test]
async fn schedules_switch_a_label_at_their_times_across_midnight() {
    let (addr, commands, _) = fake_light_server(plug_nodes(), Fake::default()).await;
    let (db, sql) = ledger();
    let home = home_link::Home::new(db, Some(format!("ws://{addr}/ws")));
    let app = home.router();
    let razor = create_label(
        &app,
        json!({ "name": "razor", "schedule": [{ "time": "00:00", "on": true }, { "time": "05:00", "on": false }] }),
    )
    .await;
    let night = create_label(
        &app,
        json!({ "name": "night", "schedule": [{ "time": "23:30", "on": true }, { "time": "01:00", "on": false }] }),
    )
    .await;
    for (vendor, serial, label) in [("Plugco", "plug-a", &razor), ("Tapo", TAPO, &night)] {
        let device = add_device(
            &app,
            &sql,
            json!({ "vendor": vendor, "serial_number": serial }),
        )
        .await;
        assert_eq!(assign(&app, &device, &[label]).await.0, StatusCode::OK);
    }

    let minute = |h: i64, m: i64| MIDNIGHT + h * 3600 + m * 60 - if h >= 12 { 86_400 } else { 0 };
    for (at, sent) in [
        (minute(23, 0), vec![]),
        (minute(23, 29), vec![]),
        (minute(23, 30), vec![(json!(5), json!(1), json!("On"))]),
        (minute(0, 0), vec![(json!(20), json!(1), json!("On"))]),
        (minute(0, 30), vec![]),
        (minute(1, 0), vec![(json!(5), json!(1), json!("Off"))]),
        (minute(4, 59), vec![]),
        (minute(5, 0), vec![(json!(20), json!(1), json!("Off"))]),
        (minute(5, 0) + 30, vec![]),
        (minute(12, 0) + 86_400, vec![]),
    ] {
        commands.lock().unwrap().clear();
        home.run_schedules(at).await;
        assert_eq!(switches(&commands), sent, "{}", at - MIDNIGHT);
    }
    let (_, labels) = call(&app, "GET", "/api/labels", None).await;
    assert_eq!(labels[0]["on"], false);
}

#[test]
fn opening_a_v0_1_36_ledger_keeps_its_labels_and_how_its_lights_behave() {
    let path = std::env::temp_dir().join(format!("home-link-v8-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    v0_1_36_ledger(&path);
    for _ in 0..2 {
        let db = home_link::open_db(path.to_str().unwrap()).unwrap();
        // Each label lights at its day's values, the dedicated one at its own;
        // every light outside a dedicated label is under the new auto label.
        assert_eq!(
            label_rows(&db),
            [
                (1, "キッチン".into(), false, Some(203), Some(5000)),
                (2, "作業".into(), false, Some(230), Some(5000)),
                (3, "仕事用デスク".into(), false, Some(254), Some(5000)),
                (4, "自動調光".into(), true, None, None),
            ]
        );
        assert_eq!(
            assignments(&db),
            [(1, vec![1, 2, 4]), (2, vec![4]), (3, vec![3]), (4, vec![4])]
        );
        let states: Vec<Option<bool>> = db
            .prepare("SELECT \"on\" FROM labels ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        // 作業 was boosted; "all on" was pressed last.
        assert_eq!(states, [None, Some(true), None, Some(true)]);
        let intent: i64 = db
            .query_row(
                "SELECT count(*) FROM settings WHERE key = 'lights_intent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(intent, 0);
    }
    let _ = std::fs::remove_file(&path);
}

/// A v0.1.36 (`user_version` 8) ledger with the schedule on and "all on" pressed last:
/// the T2 (t2-b) under キッチン (night 76, 2700 K) and the boosted 作業 (day 230),
/// the Tapo unlabelled, the desk Tapo under the dedicated 仕事用デスク (254, 5000 K)
/// and a plug unlabelled.
fn v0_1_36_ledger(path: &std::path::Path) {
    let old = rusqlite::Connection::open(path).unwrap();
    old.execute_batch(&format!(
        "CREATE TABLE rooms (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
        CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE labels (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            day_level INTEGER,
            night_level INTEGER,
            cool_kelvin INTEGER,
            warm_kelvin INTEGER
        , boost TEXT, dedicated INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE device_labels (
            device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
            label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
            PRIMARY KEY (device_id, label_id)
        );
        CREATE TABLE wifi_networks (
            id INTEGER PRIMARY KEY,
            ssid TEXT NOT NULL UNIQUE,
            password TEXT NOT NULL,
            for_devices INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE \"devices\" (
            id INTEGER PRIMARY KEY,
            room_id INTEGER REFERENCES rooms(id) ON DELETE RESTRICT,
            qr_payload TEXT UNIQUE,
            vendor TEXT,
            serial_number TEXT,
            mac TEXT UNIQUE,
            name TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            ignored INTEGER NOT NULL DEFAULT 0,
            vendor_id INTEGER,
            product_id INTEGER,
            UNIQUE (vendor, serial_number),
            CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
        );
        PRAGMA user_version = 8;
        INSERT INTO settings VALUES ('light_schedule', '{{\"enabled\":true}}'),
            ('lights_intent', '{{\"action\":\"on\",\"at\":\"2026-10-05T21:00:00+09:00\"}}');
        INSERT INTO labels (id, name, night_level, warm_kelvin, day_level, boost, dedicated) VALUES
            (1, 'キッチン', 76, 2700, NULL, NULL, 0),
            (2, '作業', NULL, NULL, 230, '[]', 0),
            (3, '仕事用デスク', NULL, NULL, 254, NULL, 1);
        UPDATE labels SET cool_kelvin = 5000 WHERE id = 3;
        INSERT INTO devices (id, vendor, serial_number, name) VALUES
            (1, 'Aqara', 't2-b', 'キッチン1'), (2, 'Tapo', '{TAPO}', '読書灯'),
            (3, 'Tapo', 'tapo-desk', 'デスク'), (4, 'Plugco', 'plug-a', '');
        INSERT INTO device_labels VALUES (1, 1), (1, 2), (3, 3);"
    ))
    .unwrap();
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn after_the_update_the_v0_1_36_widgets_and_lights_behave_as_before() {
    let path = std::env::temp_dir().join(format!("home-link-v8-app-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    v0_1_36_ledger(&path);
    let (addr, commands, _) = fake_light_server(tuning_nodes(), Fake::default()).await;
    let home = home_link::Home::new(
        home_link::open_db(path.to_str().unwrap()).unwrap(),
        Some(format!("ws://{addr}/ws")),
    )
    .with_clock(|| NIGHT);
    let app = home.router();

    // What the widget reads: each label's name and whether it is boosted.
    let (status, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        boosted(&body["labels"]),
        [
            (json!("キッチン"), json!(false)),
            (json!("作業"), json!(true)),
            (json!("仕事用デスク"), json!(false)),
            (json!("自動調光"), json!(false)),
        ]
    );
    let (_, labels) = call(&app, "GET", "/api/labels", None).await;
    assert_eq!(labels[2]["name"], "仕事用デスク");

    // The schedule still adjusts the Tapo; the T2 stays at 作業's values; the desk is left.
    home.adjust(NIGHT, "scheduled").await;
    assert_eq!(
        sent_values(&commands),
        [
            (json!(5), json!(1), json!(102)),
            (json!(5), json!(1), json!(333))
        ]
    );
    assert!(!reads(&commands).contains(&(json!(6), json!(["1/6/0"]))));

    // "All on" leaves the desk off and is not a failure; the plug is never touched.
    commands.lock().unwrap().clear();
    let (status, body) = call(&app, "POST", "/api/lights/on", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((&body["failed"], &body["switched"]), (&json!(0), &json!(3)));
    assert!(!switches(&commands).contains(&(json!(6), json!(1), json!("On"))));

    // 仕事 lights the desk at 100% and 5000 K, and puts it out again.
    commands.lock().unwrap().clear();
    let (status, body) = boost(&app, "仕事用デスク").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            &body["label"],
            &body["action"],
            &body["boosted"],
            &body["switched"]
        ),
        (
            &json!("仕事用デスク"),
            &json!("boost"),
            &json!(true),
            &json!(1)
        )
    );
    assert_eq!(switches(&commands), [(json!(6), json!(1), json!("On"))]);
    assert_eq!(
        tunings_of(&commands, 6, 1),
        [
            at_once("MoveToLevelWithOnOff", 8, 254),
            at_once("MoveToColorTemperature", 768, 200)
        ]
    );
    commands.lock().unwrap().clear();
    home.adjust(AFTER_MIDNIGHT, "scheduled").await;
    assert_eq!(tunings_of(&commands, 6, 1), []);
    commands.lock().unwrap().clear();
    let (_, body) = boost(&app, "仕事用デスク").await;
    assert_eq!(
        (&body["action"], &body["boosted"]),
        (&json!("release"), &json!(false))
    );
    assert_eq!(switches(&commands), [(json!(6), json!(1), json!("Off"))]);

    // 作業 releases the T2 to the clock's values, as it is on under 自動調光.
    commands.lock().unwrap().clear();
    let (_, body) = boost(&app, "作業").await;
    assert_eq!(
        (&body["action"], &body["boosted"]),
        (&json!("release"), &json!(false))
    );
    assert_eq!(switches(&commands), []);
    assert_eq!(tunings_of(&commands, 1, 3), released(102, 333));
    commands.lock().unwrap().clear();
    let (_, body) = boost(&app, "作業").await;
    assert_eq!(
        (&body["action"], &body["boosted"]),
        (&json!("boost"), &json!(true))
    );
    assert_eq!(
        tunings_of(&commands, 1, 3),
        [
            at_once("MoveToLevelWithOnOff", 8, 230),
            at_once("MoveToColorTemperature", 768, 200)
        ]
    );

    // "All off" puts every light out, ends every boost and the adjustment.
    commands.lock().unwrap().clear();
    let (status, body) = call(&app, "POST", "/api/lights/off", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        switches(&commands)
            .iter()
            .all(|s| s.2 == "Off" && s.0 != 20)
    );
    let (_, body) = call(&app, "GET", "/api/lights/schedule", None).await;
    assert!(boosted(&body["labels"]).iter().all(|(_, b)| b == false));
    commands.lock().unwrap().clear();
    home.adjust(NIGHT + 600, "scheduled").await;
    assert_eq!(tunings(&commands), []);

    // Unknown labels and bodies are refused as before.
    let (status, error) = boost(&app, "なし").await;
    assert_eq!(
        (status, &error["error"]),
        (StatusCode::NOT_FOUND, &json!("label_not_found"))
    );
    for bad in [
        json!({}),
        json!({ "label": 1 }),
        json!({ "label": "作業", "on": true }),
    ] {
        let (status, error) = call(&app, "POST", "/api/lights/boost", Some(bad.clone())).await;
        assert_eq!(
            (status, &error["error"]),
            (StatusCode::BAD_REQUEST, &json!("invalid_boost")),
            "{bad}"
        );
    }
    let empty = create_label(&app, json!({ "name": "空" })).await;
    assert_eq!(empty["boosted"], false);
    let (status, error) = boost(&app, "空").await;
    assert_eq!(
        (status, &error["error"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!("label_has_no_lights")
        )
    );
    let _ = std::fs::remove_file(&path);
}

/// Posts one JSON-RPC message to the MCP endpoint at `base`.
async fn mcp(client: &reqwest::Client, base: &str, message: Value) -> (u16, Value) {
    let response = client
        .post(format!("{base}/mcp"))
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .body(message.to_string())
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// The JSON text a tool call answered, and whether it was an error.
fn tool_result(reply: &Value) -> (Value, bool) {
    let result = &reply["result"];
    let text = result["content"][0]["text"].as_str().unwrap();
    (
        serde_json::from_str(text).unwrap(),
        result["isError"] == true,
    )
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn mcp_tools_manage_and_switch_labels_over_http() {
    let (addr, commands, _) = fake_light_server(plug_nodes(), Fake::default()).await;
    let (db, sql) = ledger();
    let home = home_link::Home::new(db, Some(format!("ws://{addr}/ws")));
    let plug = add_device(
        &home.router(),
        &sql,
        json!({ "vendor": "Plugco", "serial_number": "plug-a" }),
    )
    .await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = home.router();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::new();

    let (status, reply) = mcp(
        &client,
        &base,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" } } }),
    )
    .await;
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(reply["result"]["serverInfo"]["name"], "home-link");
    assert!(reply["result"]["capabilities"]["tools"].is_object());
    let (status, _) = mcp(
        &client,
        &base,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    assert_eq!(status, 202);

    let (_, reply) = mcp(
        &client,
        &base,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    let mut names: Vec<_> = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            assert_eq!(t["inputSchema"]["type"], "object", "{t}");
            t["name"].as_str().unwrap().to_owned()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "create_label",
            "delete_label",
            "label_off",
            "label_on",
            "label_toggle",
            "list_devices",
            "list_labels",
            "set_device_labels",
            "update_label",
        ]
    );

    let call_tool = |id: u64, name: &str, arguments: Value| {
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": name, "arguments": arguments } })
    };
    let (_, reply) = mcp(
        &client,
        &base,
        call_tool(
            3,
            "create_label",
            json!({ "name": "razor",
            "schedule": [{ "time": "00:00", "on": true }, { "time": "05:00", "on": false }] }),
        ),
    )
    .await;
    let (label, error) = tool_result(&reply);
    assert!(!error, "{label}");
    assert_eq!(
        label["schedule"][1],
        json!({ "time": "05:00", "on": false })
    );
    let (_, reply) = mcp(
        &client,
        &base,
        call_tool(
            4,
            "set_device_labels",
            json!({ "id": plug["id"], "label_ids": [label["id"]] }),
        ),
    )
    .await;
    let (device, error) = tool_result(&reply);
    assert!(!error, "{device}");
    assert_eq!(device["labels"][0]["name"], "razor");
    let (_, reply) = mcp(
        &client,
        &base,
        call_tool(
            5,
            "update_label",
            json!({ "id": label["id"], "name": "shaver" }),
        ),
    )
    .await;
    assert_eq!(tool_result(&reply).0["name"], "shaver");
    let (_, reply) = mcp(&client, &base, call_tool(6, "list_devices", json!({}))).await;
    assert_eq!(tool_result(&reply).0[0]["kind"], "plug");

    let (_, reply) = mcp(
        &client,
        &base,
        call_tool(7, "label_on", json!({ "id": label["id"] })),
    )
    .await;
    let (body, error) = tool_result(&reply);
    assert!(!error, "{body}");
    assert_eq!(body["switched"], 1);
    assert_eq!(switches(&commands), [(json!(20), json!(1), json!("On"))]);
    for (id, tool, command) in [(8, "label_toggle", "Off"), (9, "label_off", "Off")] {
        commands.lock().unwrap().clear();
        let (_, reply) = mcp(
            &client,
            &base,
            call_tool(id, tool, json!({ "id": label["id"] })),
        )
        .await;
        assert!(!tool_result(&reply).1);
        assert_eq!(switches(&commands), [(json!(20), json!(1), json!(command))]);
    }
    let (_, reply) = mcp(&client, &base, call_tool(10, "list_labels", json!({}))).await;
    assert_eq!(tool_result(&reply).0[0]["on"], false);

    // API errors come back as tool errors with the API's body.
    let (_, reply) = mcp(
        &client,
        &base,
        call_tool(11, "label_on", json!({ "id": 999 })),
    )
    .await;
    let (body, error) = tool_result(&reply);
    assert!(error);
    assert_eq!(body["error"], "label_not_found");
    let (_, reply) = mcp(
        &client,
        &base,
        call_tool(12, "delete_label", json!({ "id": label["id"] })),
    )
    .await;
    assert!(!tool_result(&reply).1);
    let (_, reply) = mcp(&client, &base, call_tool(13, "list_labels", json!({}))).await;
    assert_eq!(tool_result(&reply).0, json!([]));

    // Protocol errors.
    let (_, reply) = mcp(&client, &base, call_tool(14, "unlock_door", json!({}))).await;
    assert_eq!(reply["error"]["code"], -32602, "{reply}");
    let (_, reply) = mcp(&client, &base, call_tool(15, "label_on", json!({}))).await;
    assert_eq!(reply["error"]["code"], -32602, "{reply}");
    let (_, reply) = mcp(
        &client,
        &base,
        json!({ "jsonrpc": "2.0", "id": 16, "method": "resources/list" }),
    )
    .await;
    assert_eq!(reply["error"]["code"], -32601);
    let (_, reply) = mcp(
        &client,
        &base,
        json!({ "jsonrpc": "2.0", "id": 17, "method": "ping" }),
    )
    .await;
    assert_eq!(reply["result"], json!({}));
    let response = client
        .post(format!("{base}/mcp"))
        .body("{")
        .header("content-type", "application/json")
        .send()
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(reply["error"]["code"], -32700);
    let response = client.get(format!("{base}/mcp")).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 405);
}

/// A v0.1.39 (`user_version` 9) ledger as production holds it: rooms with devices in
/// them, labels with schedules and assignments, saved Wi-Fi networks and settings.
/// The schema is what v0.1.39's `open_db` leaves; the values are made up.
fn v0_1_39_ledger(path: &std::path::Path) {
    let old = rusqlite::Connection::open(path).unwrap();
    old.execute_batch(&format!(
        "CREATE TABLE rooms (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );
        CREATE TABLE settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE device_labels (
            device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
            label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
            PRIMARY KEY (device_id, label_id)
        );
        CREATE TABLE wifi_networks (
            id INTEGER PRIMARY KEY,
            ssid TEXT NOT NULL UNIQUE,
            password TEXT NOT NULL,
            for_devices INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE \"devices\" (
            id INTEGER PRIMARY KEY,
            room_id INTEGER REFERENCES rooms(id) ON DELETE RESTRICT,
            qr_payload TEXT UNIQUE,
            vendor TEXT,
            serial_number TEXT,
            mac TEXT UNIQUE,
            name TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            ignored INTEGER NOT NULL DEFAULT 0,
            vendor_id INTEGER,
            product_id INTEGER,
            UNIQUE (vendor, serial_number),
            CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
        );
        CREATE TABLE \"labels\" (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            auto INTEGER NOT NULL DEFAULT 0,
            override_level INTEGER,
            override_kelvin INTEGER,
            \"on\" INTEGER
        );
        CREATE TABLE label_schedules (
            label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
            minute INTEGER NOT NULL,
            \"on\" INTEGER NOT NULL,
            PRIMARY KEY (label_id, minute)
        );
        PRAGMA user_version = 9;
        INSERT INTO rooms VALUES (1, '部屋A'), (2, '部屋B'), (3, '部屋C'), (4, '部屋D');
        INSERT INTO settings VALUES ('light_schedule', '{{\"enabled\":true}}');
        INSERT INTO wifi_networks VALUES (1, 'fake-iot', 'fake-pass-1', 1), (2, 'fake-home', 'fake-pass-2', 0);
        INSERT INTO devices
            (id, room_id, qr_payload, vendor, serial_number, mac, name, created_at, ignored, vendor_id, product_id)
            VALUES
            (1, 1, NULL, 'Aqara', 't2-b', NULL, '灯A', '2026-10-01T00:00:00Z', 0, NULL, NULL),
            (2, 2, '{MANUAL}', 'Tapo', '{TAPO}', '00:11:22:33:44:55', '灯B', '2026-10-02T00:00:00Z', 1, NULL, NULL),
            (3, 3, '{QR}', NULL, NULL, NULL, '', '2026-10-03T00:00:00Z', 0, 65521, 32769),
            (4, NULL, NULL, 'Plugco', 'plug-a', NULL, '差込A', '2026-10-04T00:00:00Z', 0, NULL, NULL),
            (5, 4, NULL, 'Tapo', 'tapo-desk', NULL, '灯C', '2026-10-05T00:00:00Z', 0, NULL, NULL);
        INSERT INTO labels VALUES (1, '自動調光', 1, NULL, NULL, 1), (2, '固定A', 0, 254, 5000, NULL);
        INSERT INTO label_schedules VALUES (1, 0, 0), (1, 300, 1), (2, 1380, 1);
        INSERT INTO device_labels VALUES (1, 1), (2, 1), (4, 1), (5, 2), (1, 2);"
    ))
    .unwrap();
}

/// Every row of `table` as JSON text, in rowid order, for comparing before and after.
fn rows(db: &rusqlite::Connection, table: &str, columns: &str) -> Vec<String> {
    db.prepare(&format!(
        "SELECT json_array({columns}) FROM {table} ORDER BY rowid"
    ))
    .unwrap()
    .query_map([], |row| row.get(0))
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

const DEVICE_COLUMNS: &str =
    "id, qr_payload, vendor, serial_number, mac, name, created_at, ignored, vendor_id, product_id";

#[test]
fn opening_a_v0_1_39_ledger_drops_rooms_and_keeps_everything_else() {
    let path = std::env::temp_dir().join(format!("home-link-v9-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    v0_1_39_ledger(&path);
    let before = {
        let old = rusqlite::Connection::open(&path).unwrap();
        [
            rows(&old, "devices", DEVICE_COLUMNS),
            rows(
                &old,
                "labels",
                "id, name, auto, override_level, override_kelvin, \"on\"",
            ),
            rows(&old, "device_labels", "device_id, label_id"),
            rows(&old, "label_schedules", "label_id, minute, \"on\""),
            rows(&old, "wifi_networks", "id, ssid, password, for_devices"),
            rows(&old, "settings", "key, value"),
        ]
    };

    // Opening twice: the second open must not bring the rooms back.
    for _ in 0..2 {
        let db = home_link::open_db(path.to_str().unwrap()).unwrap();
        let after = [
            rows(&db, "devices", DEVICE_COLUMNS),
            rows(
                &db,
                "labels",
                "id, name, auto, override_level, override_kelvin, \"on\"",
            ),
            rows(&db, "device_labels", "device_id, label_id"),
            rows(&db, "label_schedules", "label_id, minute, \"on\""),
            rows(&db, "wifi_networks", "id, ssid, password, for_devices"),
            rows(&db, "settings", "key, value"),
        ];
        assert_eq!(after, before);
        let rooms: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'rooms'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rooms, 0);
        let columns: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('devices')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!columns.contains(&"room_id".to_owned()), "{columns:?}");
        let schema: String = db
            .query_row("SELECT group_concat(sql) FROM sqlite_schema", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!schema.contains("room"), "{schema}");
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 10);
        let broken: i64 = db
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(broken, 0);
    }

    // A device's labels still go with it, and new devices go in without a room.
    let db = home_link::open_db(path.to_str().unwrap()).unwrap();
    db.execute("DELETE FROM devices WHERE id = 1", []).unwrap();
    assert_eq!(
        rows(&db, "device_labels", "device_id, label_id"),
        ["[2,1]", "[4,1]", "[5,2]"]
    );
    db.execute(
        "INSERT INTO devices (vendor, serial_number, name) VALUES ('Tapo', 'new', '新')",
        [],
    )
    .unwrap();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn rooms_are_gone_from_the_api_and_the_mcp() {
    let (addr, _, _) = fake_light_server(home_nodes(), Fake::default()).await;
    let (app, _) = light_app(addr).await;

    for (method, uri) in [
        ("GET", "/api/rooms"),
        ("POST", "/api/rooms"),
        ("PATCH", "/api/rooms/1"),
        ("DELETE", "/api/rooms/1"),
    ] {
        let (status, body) = call(&app, method, uri, Some(json!({ "name": "部屋A" }))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        assert_eq!(body["error"], "not_found");
    }
    let (status, device) = call(&app, "GET", "/api/devices/1", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, error) = call(
        &app,
        "PATCH",
        "/api/devices/1",
        Some(json!({ "room_id": null })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_device_update");
    assert!(!error["message"].as_str().unwrap().contains("部屋"));

    let mut answers = vec![device];
    for uri in [
        "/api/devices",
        "/api/lights",
        "/api/lights/schedule",
        "/api/status",
    ] {
        let (status, body) = call(&app, "GET", uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri} {body}");
        answers.push(body);
    }
    let (status, body) = call(&app, "POST", "/api/lights/off", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    answers.push(body);
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_devices", "arguments": {} } });
    let response = app
        .clone()
        .oneshot(
            Request::post("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let reply: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let (devices, error) = tool_result(&reply);
    assert!(
        !error && devices.as_array().unwrap().len() == 3,
        "{devices}"
    );
    answers.push(devices);
    let tools = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let response = app
        .clone()
        .oneshot(
            Request::post("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(tools.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    answers.push(
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap(),
    );
    for answer in answers {
        let text = answer.to_string();
        assert!(!text.contains("room") && !text.contains("部屋"), "{text}");
    }
}
