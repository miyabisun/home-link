#![cfg(test)]

//! The light watch through the API, against a fake matterjs-server and a fake Discord webhook.

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

/// 2026-10-26 00:00 in Tokyo.
const T0: i64 = 1_792_940_400;

fn minute(n: i64) -> i64 {
    T0 + n * 60
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

/// A Discord webhook that keeps each message's content.
async fn fake_discord() -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    failing_discord(Arc::new(Mutex::new(0))).await
}

/// A Discord webhook that refuses the first `failures` messages, then keeps each one's content.
async fn failing_discord(failures: Arc<Mutex<usize>>) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let kept = messages.clone();
    let app = Router::new().route(
        "/api/webhooks/1/token",
        axum::routing::post(move |body: axum::body::Bytes| {
            let kept = kept.clone();
            let failures = failures.clone();
            async move {
                {
                    let mut failures = failures.lock().unwrap();
                    if *failures > 0 {
                        *failures -= 1;
                        return StatusCode::SERVICE_UNAVAILABLE;
                    }
                }
                let body: Value = serde_json::from_slice(&body).unwrap();
                kept.lock()
                    .unwrap()
                    .push(body["content"].as_str().unwrap().to_owned());
                StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, messages)
}

fn home(matter: SocketAddr, discord: Option<SocketAddr>) -> home_link::Home {
    home_on(":memory:", matter, discord)
}

/// A home on the database at `path`, with the three bulbs in its ledger.
fn home_on(path: &str, matter: SocketAddr, discord: Option<SocketAddr>) -> home_link::Home {
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
    let discord = discord.map(|addr| {
        home_link::discord::Discord::new(&format!("http://{addr}/api/webhooks/1/token")).unwrap()
    });
    home_link::Home::new(db, Some(format!("ws://{matter}/ws"))).with_discord(discord)
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

/// The fake matterjs-server and Discord, and a home watching through them.
struct Rig {
    matter: Arc<Mutex<Matter>>,
    messages: Arc<Mutex<Vec<String>>>,
    home: home_link::Home,
    app: Router,
}

async fn rig(unavailable: Vec<u64>, with_discord: bool) -> Rig {
    let matter = Arc::new(Mutex::new(Matter {
        unavailable,
        ..Matter::default()
    }));
    let (discord, messages) = fake_discord().await;
    let home = home(
        fake_matter(matter.clone()).await,
        with_discord.then_some(discord),
    );
    let app = home.router();
    Rig {
        matter,
        messages,
        home,
        app,
    }
}

/// At 00:00 every watched light is read and shown connected; the ignored one is not watched.
async fn every_light_is_read_and_shown(rig: &Rig) {
    let Rig {
        matter, home, app, ..
    } = rig;
    home.check_lights(minute(0)).await;
    let (status, watch) = call(app, "GET", "/api/lights/watch").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(watch["notify"], true);
    assert_eq!(watch["server_reachable"], true);
    let lights = watch["lights"].as_array().unwrap();
    assert_eq!(lights.len(), 2, "the ignored bulb is not watched: {watch}");
    assert_eq!(lights[0]["name"], "玄関");
    assert_eq!(lights[0]["network"], "wifi");
    assert_eq!(lights[0]["connected"], true);
    assert_eq!(lights[0]["last_answer"], minute(0));
    assert_eq!(asked(matter, "read_attribute"), vec![5, 7]);
}

/// The Wi-Fi bulb is out from 00:01 to 00:08: reconnected and pinged once, told after five minutes and when back.
async fn a_wifi_bulb_drops_out_and_comes_back(rig: &Rig) {
    let Rig {
        matter,
        messages,
        home,
        app,
    } = rig;
    // The Wi-Fi bulb drops out at 00:01 and is back at 00:08.
    matter.lock().unwrap().unavailable.push(5);
    for n in 1..=7 {
        home.check_lights(minute(n)).await;
    }
    assert_eq!(
        asked(matter, "interview_node"),
        vec![5],
        "reconnected once, at once"
    );
    assert_eq!(asked(matter, "ping_node"), vec![5]);
    {
        let messages = messages.lock().unwrap();
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(
            messages[0].contains("玄関（Wi-Fi）: 00:01から"),
            "{}",
            messages[0]
        );
        assert!(!messages[0].contains("物置"), "{}", messages[0]);
    }
    let (_, watch) = call(app, "GET", "/api/lights/watch").await;
    assert_eq!(watch["wifi_all_down"], true, "{watch}");
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
    home.check_lights(minute(8)).await;
    assert!(
        messages.lock().unwrap()[1].contains("玄関（Wi-Fi）: 7分つながっていませんでした"),
        "{:?}",
        messages.lock().unwrap()
    );
}

/// matterjs-server is out of reach from 00:10 to 00:16, told once instead of each light.
async fn matterjs_server_drops_out_and_comes_back(rig: &Rig) {
    let Rig {
        matter,
        messages,
        home,
        app,
    } = rig;
    // matterjs-server is out of reach from 00:10 to 00:16.
    matter.lock().unwrap().down = true;
    for n in 10..=15 {
        home.check_lights(minute(n)).await;
    }
    let (_, watch) = call(app, "GET", "/api/lights/watch").await;
    assert_eq!(watch["server_reachable"], false);
    matter.lock().unwrap().down = false;
    home.check_lights(minute(16)).await;
    {
        let messages = messages.lock().unwrap();
        assert_eq!(messages.len(), 4, "{messages:?}");
        assert!(
            messages[2].contains("matterjs-serverに5分以上届きません（00:10から）"),
            "{}",
            messages[2]
        );
        assert!(
            messages[3].contains("matterjs-serverに再び届きました"),
            "{}",
            messages[3]
        );
    }
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

/// The Thread bulb stops answering reads at 00:21; then a test message goes out, and no light was switched.
async fn a_silent_thread_bulb_is_reconnected_and_a_test_is_sent(rig: &Rig) {
    let Rig {
        matter,
        messages,
        home,
        app,
    } = rig;
    // The Thread bulb stays available but stops answering its reads.
    matter.lock().unwrap().silent.push(7);
    // Last read at 00:16, it is read at 00:21 and, having missed, again at 00:22.
    for n in 20..=22 {
        home.check_lights(minute(n)).await;
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

    let (status, sent) = call(app, "POST", "/api/lights/watch/test").await;
    assert_eq!((status, sent), (StatusCode::OK, json!({ "sent": true })));
    assert!(
        messages
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("試しの知らせ")
    );
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
async fn the_watch_reconnects_tells_and_keeps_the_history_of_a_light_and_of_matterjs_server() {
    let rig = rig(vec![16], true).await;
    every_light_is_read_and_shown(&rig).await;
    a_wifi_bulb_drops_out_and_comes_back(&rig).await;
    matterjs_server_drops_out_and_comes_back(&rig).await;
    the_history_holds_both_stretches(&rig).await;
    a_silent_thread_bulb_is_reconnected_and_a_test_is_sent(&rig).await;
}

#[tokio::test]
async fn without_a_destination_the_watch_tells_nothing_and_keeps_the_rest() {
    let matter = Arc::new(Mutex::new(Matter {
        unavailable: vec![5],
        ..Matter::default()
    }));
    let home = home(fake_matter(matter.clone()).await, None);
    let app = home.router();
    for n in 0..=6 {
        home.check_lights(minute(n)).await;
    }
    assert_eq!(asked(&matter, "interview_node"), vec![5]);
    let (_, watch) = call(&app, "GET", "/api/lights/watch").await;
    assert_eq!(watch["notify"], false);
    assert_eq!(watch["lights"][0]["connected"], false);
    let (_, outages) = call(&app, "GET", "/api/lights/outages").await;
    assert_eq!(outages["outages"][0]["started_at"], minute(0));
    let (status, error) = call(&app, "POST", "/api/lights/watch/test").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"], "notify_not_configured");
}

#[tokio::test]
async fn a_notice_the_destination_refused_is_sent_at_the_next_check() {
    let matter = Arc::new(Mutex::new(Matter {
        unavailable: vec![5],
        ..Matter::default()
    }));
    let failures = Arc::new(Mutex::new(1));
    let (discord, messages) = failing_discord(failures.clone()).await;
    let home = home(fake_matter(matter.clone()).await, Some(discord));
    for n in 0..=5 {
        home.check_lights(minute(n)).await;
    }
    assert_eq!(*failures.lock().unwrap(), 0, "the down notice was refused");
    assert!(messages.lock().unwrap().is_empty());
    home.check_lights(minute(6)).await;
    let messages = messages.lock().unwrap();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(
        messages[0].contains("玄関（Wi-Fi）: 00:00から"),
        "{}",
        messages[0]
    );
}

#[tokio::test]
async fn after_a_restart_a_light_told_down_is_not_told_again_and_is_told_back() {
    let matter = Arc::new(Mutex::new(Matter {
        unavailable: vec![5],
        ..Matter::default()
    }));
    let (discord, messages) = fake_discord().await;
    let addr = fake_matter(matter.clone()).await;
    let path = format!(
        "file:watch-restart-{}?mode=memory&cache=shared",
        std::process::id()
    );
    let before = home_on(&path, addr, Some(discord));
    for n in 0..=5 {
        before.check_lights(minute(n)).await;
    }
    assert_eq!(messages.lock().unwrap().len(), 1);

    // home-link restarts, as Watchtower does on a release, while the bulb is still down.
    let after = home_on(&path, addr, Some(discord));
    after.resume_watch();
    for n in 20..=26 {
        after.check_lights(minute(n)).await;
    }
    assert_eq!(messages.lock().unwrap().len(), 1, "not told down again");
    matter.lock().unwrap().unavailable.clear();
    after.check_lights(minute(27)).await;
    let messages = messages.lock().unwrap();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(
        messages[1].contains("27分つながっていませんでした"),
        "{}",
        messages[1]
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

/// The Thread bulb leaves the fabric at 00:00:40, as matterjs-server sends: told once, not reconnected.
async fn a_thread_bulb_leaves_the_fabric(rig: &Rig) {
    let Rig {
        matter,
        messages,
        home,
        ..
    } = rig;
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
    {
        let messages = messages.lock().unwrap();
        assert_eq!(messages.len(), 1, "{messages:?}");
        for part in [
            "網（fabric）から外れました",
            "寝室（Thread）",
            "製造番号 thread-bulb",
            "10-26 00:00から（matterjs-serverが外した時刻）",
            "home-linkアプリで登録し直す",
        ] {
            assert!(messages[0].contains(part), "{part}: {}", messages[0]);
        }
    }
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

/// Registered again, it is served under a new node and found on its ledger row: told back once.
async fn the_bulb_registered_again_is_back(rig: &Rig) {
    let Rig {
        matter,
        messages,
        home,
        app,
    } = rig;
    // Registered again: matterjs-server serves it under a new node, found by its serial number.
    {
        let mut matter = matter.lock().unwrap();
        matter.gone.clear();
        matter.renumbered.push((7, 70));
    }
    home.check_lights(minute(10)).await;
    {
        let messages = messages.lock().unwrap();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(
            messages[1].contains("寝室（Thread）: 9分つながっていませんでした"),
            "{}",
            messages[1]
        );
    }
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
async fn a_ledger_bulb_that_leaves_the_fabric_is_told_and_comes_back_when_registered_again() {
    let matter = Arc::new(Mutex::new(Matter {
        unavailable: vec![16],
        ..Matter::default()
    }));
    let (discord, messages) = fake_discord().await;
    // matterjs-server removes the Thread bulb's node at 00:00:40.
    let home = home(fake_matter(matter.clone()).await, Some(discord)).with_clock(|| T0 + 40);
    let app = home.router();
    let rig = Rig {
        matter,
        messages,
        home,
        app,
    };
    a_thread_bulb_leaves_the_fabric(&rig).await;
    the_bulb_that_left_is_shown_everywhere(&rig).await;
    the_lights_and_devices_show_the_bulb_that_left(&rig).await;
    the_bulb_registered_again_is_back(&rig).await;
}
