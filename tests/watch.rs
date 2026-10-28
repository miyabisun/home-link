#![cfg(test)]

//! The light watch through the API, against a fake matterjs-server.

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
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

/// 2026-10-26 00:00 in Tokyo.
const T0: i64 = 1_792_940_400;

fn minute(n: i64) -> i64 {
    T0 + n * 60
}

/// The time the home of the watch's main test reads, set as its checks go.
static NOW: AtomicI64 = AtomicI64::new(T0);

fn now() -> i64 {
    NOW.load(Ordering::SeqCst)
}

/// Runs the check of minute `n` with the home's clock at it.
async fn check_at(home: &home_link::Home, n: i64) {
    NOW.store(minute(n), Ordering::SeqCst);
    home.check_lights(minute(n)).await;
}

async fn call(app: &Router, method: &str, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// What the fake matterjs-server serves and what it was asked.
#[derive(Default)]
struct Matter {
    /// Out of reach: every connection is dropped before the upgrade.
    down: bool,
    /// Node ids served as not available.
    unavailable: Vec<u64>,
    /// Node ids whose reads are never answered.
    silent: Vec<u64>,
    /// Every command but `get_nodes`, by name and node.
    asked: Vec<(String, u64)>,
    /// Node ids no longer served: their bulbs left the fabric.
    gone: Vec<u64>,
    /// Node ids served under another, as a bulb registered again is.
    renumbered: Vec<(u64, u64)>,
    /// Whether a connection listens for events, and the events still to send it.
    listening: bool,
    events: Vec<Value>,
}

/// A Wi-Fi bulb, a Thread bulb (a child of the router 0x4c00) and a Wi-Fi bulb the ledger ignores.
fn nodes(matter: &Matter) -> Value {
    let node = |id: u64, serial: &str, network: u64| {
        let served = matter
            .renumbered
            .iter()
            .find(|(from, _)| *from == id)
            .map_or(id, |(_, to)| *to);
        let mut node = json!({ "node_id": served, "available": !matter.unavailable.contains(&id), "attributes": {
            "0/40/1": "Repro", "0/40/15": serial, "0/40/3": "Bulb", "0/49/65532": network,
            "0/40/10": "1.0.0.6", "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": true,
        }});
        if network == 2 {
            node["attributes"]["0/53/64"] = json!(0x4c01);
        }
        (!matter.gone.contains(&id)).then_some(node)
    };
    [
        node(5, "wifi-bulb", 1),
        node(7, "thread-bulb", 2),
        node(16, "dead-bulb", 1),
    ]
    .into_iter()
    .flatten()
    .collect()
}

async fn fake_matter(matter: Arc<Mutex<Matter>>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            if matter.lock().unwrap().down {
                continue;
            }
            let matter = matter.clone();
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let info = json!({ "fabric_id": 1, "schema_version": 13 });
                if ws.send(Message::text(info.to_string())).await.is_err() {
                    return;
                }
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let command: Value = serde_json::from_str(&text).unwrap();
                    let id = command["message_id"].clone();
                    let name = command["command"].as_str().unwrap().to_owned();
                    let node = command["args"]["node_id"].as_u64().unwrap_or_default();
                    let reply = {
                        let mut matter = matter.lock().unwrap();
                        if name != "get_nodes" {
                            matter.asked.push((name.clone(), node));
                        }
                        match name.as_str() {
                            "get_nodes" => json!({ "message_id": id, "result": nodes(&matter) }),
                            "start_listening" => {
                                matter.listening = true;
                                json!({ "message_id": id, "result": nodes(&matter) })
                            }
                            "read_attribute" if matter.silent.contains(&node) => continue,
                            "read_attribute" => {
                                json!({ "message_id": id, "result": { "1/6/0": true } })
                            }
                            "interview_node" => json!({ "message_id": id, "result": null }),
                            // The bulb is off the network: its address does not answer.
                            "ping_node" => {
                                json!({ "message_id": id, "result": { "fd00::5": false } })
                            }
                            other => panic!("unexpected command {other}"),
                        }
                    };
                    if ws.send(Message::text(reply.to_string())).await.is_err() {
                        return;
                    }
                    if name == "start_listening" {
                        loop {
                            let events = std::mem::take(&mut matter.lock().unwrap().events);
                            for event in events {
                                if ws.send(Message::text(event.to_string())).await.is_err() {
                                    return;
                                }
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                    }
                }
            });
        }
    });
    addr
}

fn home(matter: SocketAddr) -> home_link::Home {
    home_on(":memory:", matter)
}

/// A home on the database at `path`, with the three bulbs in its ledger.
fn home_on(path: &str, matter: SocketAddr) -> home_link::Home {
    let db = home_link::open_db(path).unwrap();
    for (serial, name, ignored) in [
        ("wifi-bulb", "玄関", false),
        ("thread-bulb", "寝室", false),
        ("dead-bulb", "物置", true),
    ] {
        db.execute(
            "INSERT OR IGNORE INTO devices (vendor, serial_number, name, ignored) VALUES ('Repro', ?1, ?2, ?3)",
            rusqlite::params![serial, name, ignored],
        )
        .unwrap();
    }
    home_link::Home::new(db, Some(format!("ws://{matter}/ws")))
}

fn asked(matter: &Arc<Mutex<Matter>>, name: &str) -> Vec<u64> {
    matter
        .lock()
        .unwrap()
        .asked
        .iter()
        .filter(|(asked, _)| asked == name)
        .map(|(_, node)| *node)
        .collect()
}

/// What asks for the user now, as the admin screen reads it.
async fn attention(app: &Router) -> Value {
    let (status, body) = call(app, "GET", "/api/attention").await;
    assert_eq!(status, StatusCode::OK);
    body["items"].clone()
}

/// The fake matterjs-server, and a home watching through it.
struct Rig {
    matter: Arc<Mutex<Matter>>,
    home: home_link::Home,
    app: Router,
}

async fn rig(unavailable: Vec<u64>) -> Rig {
    let matter = Arc::new(Mutex::new(Matter {
        unavailable,
        ..Matter::default()
    }));
    let home = home(fake_matter(matter.clone()).await).with_clock(now);
    let app = home.router();
    Rig { matter, home, app }
}

/// At 00:00 every watched light is read and shown connected; the ignored one is not watched.
async fn every_light_is_read_and_shown(rig: &Rig) {
    let Rig { matter, home, app } = rig;
    check_at(home, 0).await;
    let (status, watch) = call(app, "GET", "/api/lights/watch").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(watch.get("notify"), None, "nothing is sent anywhere");
    assert_eq!(watch["server_reachable"], true);
    let lights = watch["lights"].as_array().unwrap();
    assert_eq!(lights.len(), 2, "the ignored bulb is not watched: {watch}");
    assert_eq!(lights[0]["name"], "玄関");
    assert_eq!(lights[0]["network"], "wifi");
    assert_eq!(lights[0]["connected"], true);
    assert_eq!(lights[0]["last_answer"], minute(0));
    assert_eq!(asked(matter, "read_attribute"), vec![5, 7]);
}

/// The Wi-Fi bulb is out from 00:01 to 00:08: reconnected and pinged once, shown down, and
/// asking nothing of the user while home-link brings it back.
async fn a_wifi_bulb_drops_out_and_comes_back(rig: &Rig) {
    let Rig { matter, home, app } = rig;
    // The Wi-Fi bulb drops out at 00:01 and is back at 00:08.
    matter.lock().unwrap().unavailable.push(5);
    for n in 1..=7 {
        check_at(home, n).await;
    }
    assert_eq!(
        asked(matter, "interview_node"),
        vec![5],
        "reconnected once, at once"
    );
    assert_eq!(asked(matter, "ping_node"), vec![5]);
    assert_eq!(attention(app).await, json!([]), "one reconnection so far");
    let (_, watch) = call(app, "GET", "/api/lights/watch").await;
    assert_eq!(watch["wifi_all_down"], true, "{watch}");
    assert_eq!(
        (&watch["lights"][0]["state"], &watch["lights"][0]["since"]),
        (&json!("down"), &json!(minute(1)))
    );
    assert_eq!(watch["lights"][0]["outage"]["reachable"], false);
    let (_, at) = call(
        app,
        "GET",
        &format!("/api/lights/connected?at={}", minute(3) + 20),
    )
    .await;
    assert_eq!(at["checked_at"], minute(3));
    assert_eq!(at["lights"][0]["connected"], false);
    assert_eq!(at["lights"][1]["connected"], true);

    matter.lock().unwrap().unavailable.retain(|node| *node != 5);
    check_at(home, 8).await;
    let (_, watch) = call(app, "GET", "/api/lights/watch").await;
    assert_eq!(watch["lights"][0]["state"], "connected");
}

/// matterjs-server is out of reach from 00:10 to 00:16: after five minutes it asks for the
/// user, once instead of each light, until it answers.
async fn matterjs_server_drops_out_and_comes_back(rig: &Rig) {
    let Rig { matter, home, app } = rig;
    // matterjs-server is out of reach from 00:10 to 00:16.
    matter.lock().unwrap().down = true;
    for n in 10..=14 {
        check_at(home, n).await;
    }
    assert_eq!(attention(app).await, json!([]), "four minutes");
    check_at(home, 15).await;
    assert_eq!(
        attention(app).await,
        json!([{ "kind": "server_down", "since": minute(10) }])
    );
    let (_, watch) = call(app, "GET", "/api/lights/watch").await;
    assert_eq!(watch["server_reachable"], false);
    matter.lock().unwrap().down = false;
    check_at(home, 16).await;
    assert_eq!(attention(app).await, json!([]), "back");
    let (_, at) = call(
        app,
        "GET",
        &format!("/api/lights/connected?at={}", minute(12)),
    )
    .await;
    assert_eq!(at["server_reachable"], false);
    assert_eq!(
        at["lights"][0]["connected"], false,
        "unknown while out of reach: {at}"
    );
}

/// The history holds both stretches with what was done about them.
async fn the_history_holds_both_stretches(rig: &Rig) {
    let Rig { app, .. } = rig;
    let (status, outages) = call(app, "GET", "/api/lights/outages").await;
    assert_eq!(status, StatusCode::OK);
    let outages = outages["outages"].as_array().unwrap();
    assert_eq!(outages.len(), 2, "{outages:?}");
    assert_eq!(
        (
            &outages[0]["name"],
            &outages[0]["network"],
            &outages[0]["found_by"]
        ),
        (&json!("玄関"), &json!("wifi"), &json!("unavailable"))
    );
    assert_eq!(
        (&outages[0]["started_at"], &outages[0]["ended_at"]),
        (&json!(minute(1)), &json!(minute(8)))
    );
    assert_eq!(outages[0]["reconnects"], 1);
    assert_eq!(outages[0]["reconnect_result"], "accepted");
    assert_eq!(outages[0]["reachable"], false);
    assert_eq!(
        (&outages[1]["node_id"], &outages[1]["found_by"]),
        (&Value::Null, &json!("unreachable"))
    );
    assert_eq!(
        (&outages[1]["started_at"], &outages[1]["ended_at"]),
        (&json!(minute(10)), &json!(minute(16)))
    );
    let (_, one) = call(app, "GET", "/api/lights/outages?node_id=7&endpoint=1").await;
    assert_eq!(one["outages"], json!([]));
    let (status, _) = call(app, "GET", "/api/lights/outages?node_id=7").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The Thread bulb stops answering reads at 00:21: reconnected, and after three reconnections
/// it asks to be switched off and on at the wall. No light was switched.
async fn a_silent_thread_bulb_is_reconnected_then_asks_for_the_wall_switch(rig: &Rig) {
    let Rig { matter, home, app } = rig;
    // The Thread bulb stays available but stops answering its reads.
    matter.lock().unwrap().silent.push(7);
    // Last read at 00:16, it is read at 00:21 and, having missed, again at 00:22.
    for n in 20..=22 {
        check_at(home, n).await;
    }
    assert_eq!(
        asked(matter, "interview_node"),
        vec![5, 7],
        "two misses in a row"
    );
    assert_eq!(
        asked(matter, "ping_node"),
        vec![5],
        "a Thread bulb is not pinged"
    );

    // From 00:23 it is served as not available, as a bulb switched off is: no read waits on it.
    {
        let mut matter = matter.lock().unwrap();
        matter.silent.clear();
        matter.unavailable.push(7);
    }
    // Reconnected at 00:22, 00:32 and 00:42; the check at 00:43 still finds it down.
    for n in 23..=42 {
        check_at(home, n).await;
    }
    assert_eq!(
        attention(app).await,
        json!([]),
        "the third reconnection is in"
    );
    check_at(home, 43).await;
    assert_eq!(
        attention(app).await,
        json!([{ "kind": "power_cycle", "name": "寝室", "device_id": 2, "since": minute(21) }])
    );
    let (status, _) = call(app, "POST", "/api/lights/watch/test").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        asked(matter, "device_command"),
        Vec::<u64>::new(),
        "no light was switched"
    );
    let (status, _) = call(
        app,
        "GET",
        &format!("/api/lights/connected?at={}", minute(-60)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_watch_reconnects_shows_and_keeps_the_history_of_a_light_and_of_matterjs_server() {
    let rig = rig(vec![16]).await;
    every_light_is_read_and_shown(&rig).await;
    a_wifi_bulb_drops_out_and_comes_back(&rig).await;
    matterjs_server_drops_out_and_comes_back(&rig).await;
    the_history_holds_both_stretches(&rig).await;
    a_silent_thread_bulb_is_reconnected_then_asks_for_the_wall_switch(&rig).await;
}

#[tokio::test]
async fn after_a_restart_a_light_down_keeps_its_stretch_until_it_is_back() {
    let matter = Arc::new(Mutex::new(Matter {
        unavailable: vec![5],
        ..Matter::default()
    }));
    let addr = fake_matter(matter.clone()).await;
    let path = format!(
        "file:watch-restart-{}?mode=memory&cache=shared",
        std::process::id()
    );
    let before = home_on(&path, addr);
    for n in 0..=5 {
        before.check_lights(minute(n)).await;
    }

    // home-link restarts, as Watchtower does on a release, while the bulb is still down.
    let after = home_on(&path, addr);
    after.resume_watch();
    let (_, watch) = call(&after.router(), "GET", "/api/lights/watch").await;
    assert_eq!(
        (&watch["lights"][0]["state"], &watch["lights"][0]["since"]),
        (&json!("down"), &json!(minute(0))),
        "taken back: {watch}"
    );
    for n in 20..=26 {
        after.check_lights(minute(n)).await;
    }
    matter.lock().unwrap().unavailable.clear();
    after.check_lights(minute(27)).await;
    let (_, outages) = call(&after.router(), "GET", "/api/lights/outages").await;
    let outages = outages["outages"].as_array().unwrap();
    assert_eq!(
        outages.len(),
        1,
        "one stretch across the restart: {outages:?}"
    );
    assert_eq!(
        (&outages[0]["started_at"], &outages[0]["ended_at"]),
        (&json!(minute(0)), &json!(minute(27)))
    );
    drop(before);
}

/// Waits until `done` holds, for up to two seconds.
async fn until(done: impl Fn() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("not done in time");
}

/// The Thread bulb leaves the fabric at 00:00:40, as matterjs-server sends: not reconnected.
async fn a_thread_bulb_leaves_the_fabric(rig: &Rig) {
    let Rig { matter, home, .. } = rig;
    home.check_lights(minute(0)).await;
    tokio::spawn(home.clone().watch_removals());
    until(|| matter.lock().unwrap().listening).await;
    {
        let mut matter = matter.lock().unwrap();
        matter.gone.push(7);
        matter
            .events
            .push(json!({ "event": "node_removed", "data": 7 }));
    }
    until(|| matter.lock().unwrap().events.is_empty()).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    for n in 1..=6 {
        home.check_lights(minute(n)).await;
    }

    assert_eq!(
        asked(matter, "interview_node"),
        Vec::<u64>::new(),
        "no node to reconnect"
    );
}

/// The watch and its history show the bulb that left, with what was last known of it.
async fn the_bulb_that_left_is_shown_everywhere(rig: &Rig) {
    let Rig { app, .. } = rig;
    let (_, watch) = call(app, "GET", "/api/lights/watch").await;
    assert_eq!(watch["wifi_all_down"], false);
    let lights = watch["lights"].as_array().unwrap();
    let left = lights
        .iter()
        .find(|l| l["name"] == "寝室")
        .expect("still watched");
    assert_eq!(
        (
            &left["state"],
            &left["connected"],
            &left["node_id"],
            &left["network"],
            &left["device_id"]
        ),
        (
            &json!("left_fabric"),
            &json!(false),
            &Value::Null,
            &Value::Null,
            &json!(2)
        ),
        "{left}"
    );
    assert_eq!(
        lights.iter().find(|l| l["name"] == "玄関").unwrap()["state"],
        "connected"
    );
    let outage = &left["outage"];
    assert_eq!(
        (
            &outage["found_by"],
            &outage["left_by"],
            &outage["left_at"],
            &outage["started_at"]
        ),
        (
            &json!("left_fabric"),
            &json!("event"),
            &json!(T0 + 40),
            &json!(T0 + 40)
        )
    );

    let (_, outages) = call(app, "GET", "/api/lights/outages").await;
    let row = &outages["outages"][0];
    assert_eq!(
        (
            &row["node_id"],
            &row["device_id"],
            &row["serial_number"],
            &row["network"]
        ),
        (
            &json!(7),
            &json!(2),
            &json!("thread-bulb"),
            &json!("thread")
        ),
        "{outages}"
    );
    assert_eq!(
        (
            &row["left_at"],
            &row["left_by"],
            &row["software_version"],
            &row["thread_parent"]
        ),
        (
            &json!(T0 + 40),
            &json!("event"),
            &json!("1.0.0.6"),
            &json!("0x4c00")
        )
    );
}

/// The minute's check, the lights and the devices show the bulb that left, with when it left.
async fn the_lights_and_devices_show_the_bulb_that_left(rig: &Rig) {
    let Rig { app, .. } = rig;
    let (_, at) = call(
        app,
        "GET",
        &format!("/api/lights/connected?at={}", minute(3)),
    )
    .await;
    let gone = at["lights"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "寝室")
        .cloned();
    assert_eq!(
        gone,
        Some(
            json!({ "node_id": null, "endpoint": null, "name": "寝室", "connected": false,
            "device_id": 2, "state": "left_fabric" })
        )
    );

    let (_, all) = call(app, "GET", "/api/lights").await;
    assert_eq!(
        all["missing_devices"],
        json!([{ "id": 2, "name": "寝室", "state": "left_fabric", "left_at": T0 + 40 }])
    );
    let (_, devices) = call(app, "GET", "/api/devices").await;
    let device = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == 2)
        .cloned()
        .unwrap();
    assert_eq!(
        (&device["state"], &device["left_fabric"], &device["left_at"]),
        (&json!("unconnected"), &json!(true), &json!(T0 + 40))
    );
}

/// Registered again, it is served under a new node and found on its ledger row.
async fn the_bulb_registered_again_is_back(rig: &Rig) {
    let Rig { matter, home, app } = rig;
    // Registered again: matterjs-server serves it under a new node, found by its serial number.
    {
        let mut matter = matter.lock().unwrap();
        matter.gone.clear();
        matter.renumbered.push((7, 70));
    }
    home.check_lights(minute(10)).await;
    let (_, watch) = call(app, "GET", "/api/lights/watch").await;
    let back: Vec<_> = watch["lights"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["name"] == "寝室")
        .collect();
    assert_eq!(back.len(), 1, "{watch}");
    assert_eq!(
        (&back[0]["node_id"], &back[0]["state"]),
        (&json!(70), &json!("connected"))
    );
    let (_, outages) = call(app, "GET", "/api/lights/outages").await;
    assert_eq!(outages["outages"][0]["ended_at"], minute(10));
}

#[tokio::test]
async fn a_ledger_bulb_that_leaves_the_fabric_is_shown_and_comes_back_when_registered_again() {
    let matter = Arc::new(Mutex::new(Matter {
        unavailable: vec![16],
        ..Matter::default()
    }));
    // matterjs-server removes the Thread bulb's node at 00:00:40.
    let home = home(fake_matter(matter.clone()).await).with_clock(|| T0 + 40);
    let app = home.router();
    let rig = Rig { matter, home, app };
    a_thread_bulb_leaves_the_fabric(&rig).await;
    the_bulb_that_left_is_shown_everywhere(&rig).await;
    the_lights_and_devices_show_the_bulb_that_left(&rig).await;
    the_bulb_registered_again_is_back(&rig).await;
}
