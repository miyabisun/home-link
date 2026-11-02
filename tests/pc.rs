#![cfg(test)]

//! PCs that keep a WebSocket open to home-link, and the sleep and wake sent down it.

use std::{net::SocketAddr, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const TIMES: home_link::pc::Times = home_link::pc::Times {
    hello: Duration::from_millis(500),
    ping: Duration::from_millis(100),
    idle: Duration::from_millis(400),
    answer: Duration::from_millis(200),
};

/// home-link on a port with the short `TIMES`; returns its address.
async fn serve() -> SocketAddr {
    let home =
        home_link::Home::new(home_link::open_db(":memory:").unwrap(), None).with_pc_times(TIMES);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = home.router();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

async fn open(addr: SocketAddr) -> Socket {
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/pcs/connect"))
        .await
        .unwrap();
    socket
}

/// A PC that has said hello as `name`.
async fn join(addr: SocketAddr, hello: Value) -> Socket {
    let mut socket = open(addr).await;
    socket.send(Message::text(hello.to_string())).await.unwrap();
    // home-link takes the hello before it lists the PC.
    let name = hello["name"].as_str().unwrap_or_default().to_owned();
    for _ in 0..50 {
        if names(addr).await.contains(&name) {
            return socket;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{name} was never listed");
}

async fn call(addr: SocketAddr, method: &str, path: &str) -> (u16, Value) {
    let response = reqwest::Client::new()
        .request(method.parse().unwrap(), format!("http://{addr}{path}"))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn list(addr: SocketAddr) -> Value {
    let (status, pcs) = call(addr, "GET", "/api/pcs").await;
    assert_eq!(status, 200, "{pcs}");
    pcs
}

async fn names(addr: SocketAddr) -> Vec<String> {
    list(addr)
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|pc| pc["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The next text message, answering pings on the way as a PC's client does.
async fn next_text(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("a message within 2 s")
            .expect("the socket open")
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// Reads until home-link closes the socket; true when it does within 2 s.
async fn closed(socket: &mut Socket) -> bool {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                None | Some(Err(_) | Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .is_ok()
}

#[tokio::test]
async fn lists_each_pc_by_name_with_what_it_reports() {
    let addr = serve().await;
    assert_eq!(list(addr).await, json!([]));

    let _work = join(
        addr,
        json!({ "type": "hello", "name": "work", "sleeping": false }),
    )
    .await;
    let _game = join(addr, json!({ "type": "hello", "name": "game" })).await;

    assert_eq!(
        list(addr).await,
        json!([
            { "name": "game", "sleeping": null },
            { "name": "work", "sleeping": false },
        ])
    );
}

#[tokio::test]
async fn sends_sleep_and_wake_to_the_named_pc_and_returns_its_answer() {
    let addr = serve().await;
    let mut work = join(
        addr,
        json!({ "type": "hello", "name": "work", "sleeping": false }),
    )
    .await;
    let mut game = join(
        addr,
        json!({ "type": "hello", "name": "game", "sleeping": false }),
    )
    .await;

    for (action, sleeping) in [("sleep", true), ("wake", false)] {
        let path = format!("/api/pcs/work/{action}");
        let press = tokio::spawn(async move { call(addr, "POST", &path).await });
        let command = next_text(&mut work).await;
        assert_eq!(command["type"], "command", "{command}");
        assert_eq!(command["action"], action, "{command}");
        work.send(Message::text(
            json!({ "type": "result", "id": command["id"], "ok": true, "sleeping": sleeping })
                .to_string(),
        ))
        .await
        .unwrap();

        let (status, body) = press.await.unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body, json!({ "status": "accepted", "sleeping": sleeping }));
        assert_eq!(
            list(addr).await,
            json!([
                { "name": "game", "sleeping": false },
                { "name": "work", "sleeping": sleeping },
            ])
        );
    }
    // The other PC heard nothing but pings.
    let quiet = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Some(Ok(Message::Text(text))) = game.next().await {
                return text;
            }
        }
    })
    .await;
    assert!(quiet.is_err(), "game got {quiet:?}");
}

#[tokio::test]
async fn takes_the_state_a_pc_reports_on_its_own() {
    let addr = serve().await;
    let mut work = join(
        addr,
        json!({ "type": "hello", "name": "work", "sleeping": false }),
    )
    .await;

    work.send(Message::text(
        json!({ "type": "state", "sleeping": true }).to_string(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        list(addr).await,
        json!([{ "name": "work", "sleeping": true }])
    );
}

#[tokio::test]
async fn names_the_failure_when_a_pc_cannot_be_reached_or_refuses() {
    let addr = serve().await;

    let (status, body) = call(addr, "POST", "/api/pcs/work/sleep").await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], "pc_not_connected", "{body}");

    let mut work = join(addr, json!({ "type": "hello", "name": "work" })).await;

    // Refused, with the PC's reason.
    let press = tokio::spawn(call(addr, "POST", "/api/pcs/work/sleep"));
    let command = next_text(&mut work).await;
    work.send(Message::text(
        json!({ "type": "result", "id": command["id"], "ok": false, "message": "no monitor" })
            .to_string(),
    ))
    .await
    .unwrap();
    let (status, body) = press.await.unwrap();
    assert_eq!(status, 502, "{body}");
    assert_eq!(body["error"], "pc_rejected", "{body}");
    assert!(
        body["message"].as_str().unwrap().contains("no monitor"),
        "{body}"
    );

    // No answer within `TIMES.answer`, while the socket stays open.
    let press = tokio::spawn(call(addr, "POST", "/api/pcs/work/wake"));
    let command = next_text(&mut work).await;
    let (status, body) = press.await.unwrap();
    assert_eq!(status, 504, "{body}");
    assert_eq!(body["error"], "pc_no_answer", "{body}");
    // A late answer is dropped and the PC stays connected.
    work.send(Message::text(
        json!({ "type": "result", "id": command["id"], "ok": true }).to_string(),
    ))
    .await
    .unwrap();
    assert_eq!(names(addr).await, ["work"]);

    // The socket closes while home-link waits.
    let press = tokio::spawn(call(addr, "POST", "/api/pcs/work/sleep"));
    next_text(&mut work).await;
    drop(work);
    let (status, body) = press.await.unwrap();
    assert_eq!(status, 502, "{body}");
    assert_eq!(body["error"], "pc_disconnected", "{body}");
    assert_eq!(list(addr).await, json!([]));
}

#[tokio::test]
async fn a_pc_connecting_again_under_its_name_replaces_the_old_socket() {
    let addr = serve().await;
    let mut old = join(
        addr,
        json!({ "type": "hello", "name": "work", "sleeping": false }),
    )
    .await;
    let mut new = join(
        addr,
        json!({ "type": "hello", "name": "work", "sleeping": true }),
    )
    .await;

    assert!(closed(&mut old).await, "the old socket stays open");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        list(addr).await,
        json!([{ "name": "work", "sleeping": true }])
    );

    let press = tokio::spawn(call(addr, "POST", "/api/pcs/work/wake"));
    let command = next_text(&mut new).await;
    new.send(Message::text(
        json!({ "type": "result", "id": command["id"], "ok": true }).to_string(),
    ))
    .await
    .unwrap();
    let (status, body) = press.await.unwrap();
    assert_eq!(status, 200, "{body}");
    // Without a reported state, the PC's last one stands.
    assert_eq!(body, json!({ "status": "accepted", "sleeping": true }));
}

#[tokio::test]
async fn closes_a_socket_without_a_valid_hello() {
    let addr = serve().await;

    // Nothing said within `TIMES.hello`.
    let mut silent = open(addr).await;
    assert!(closed(&mut silent).await, "a silent socket stays open");

    for hello in [
        json!({ "type": "hello", "name": "  " }),
        json!({ "type": "hello", "name": "x".repeat(101) }),
        json!({ "type": "state", "sleeping": true }),
        json!("work"),
    ] {
        let mut socket = open(addr).await;
        socket.send(Message::text(hello.to_string())).await.unwrap();
        let error = next_text(&mut socket).await;
        assert_eq!(error["type"], "error", "{hello}: {error}");
        assert!(closed(&mut socket).await, "{hello} left the socket open");
    }
    assert_eq!(list(addr).await, json!([]));
}

#[tokio::test]
async fn drops_a_pc_that_stops_answering_pings_and_keeps_one_that_answers() {
    let addr = serve().await;
    // tungstenite answers pings only while the socket is read.
    let mut alive = join(addr, json!({ "type": "hello", "name": "alive" })).await;
    let _stuck = join(addr, json!({ "type": "hello", "name": "stuck" })).await;

    let open = tokio::time::timeout(Duration::from_millis(1200), async {
        while let Some(Ok(_)) = alive.next().await {}
    })
    .await
    .is_err();
    assert!(open, "alive was closed");
    assert_eq!(names(addr).await, ["alive"]);
    drop(alive);
}

#[tokio::test]
async fn mcp_lists_pcs_and_sends_sleep_and_wake() {
    let addr = serve().await;
    let mut work = join(
        addr,
        json!({ "type": "hello", "name": "work", "sleeping": false }),
    )
    .await;
    let mcp = |name: &'static str, arguments: Value| async move {
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": arguments } });
        let text = reqwest::Client::new()
            .post(format!("http://{addr}/mcp"))
            .header("content-type", "application/json")
            .body(request.to_string())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let reply: Value = serde_json::from_str(&text).unwrap();
        let result = &reply["result"];
        (
            result["isError"].as_bool().unwrap(),
            serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap()).unwrap(),
        )
    };

    assert_eq!(
        mcp("list_pcs", json!({})).await,
        (false, json!([{ "name": "work", "sleeping": false }]))
    );

    let press = tokio::spawn(mcp("sleep_pc", json!({ "name": "work" })));
    let command = next_text(&mut work).await;
    assert_eq!(command["action"], "sleep", "{command}");
    work.send(Message::text(
        json!({ "type": "result", "id": command["id"], "ok": true, "sleeping": true }).to_string(),
    ))
    .await
    .unwrap();
    assert_eq!(
        press.await.unwrap(),
        (false, json!({ "status": "accepted", "sleeping": true }))
    );

    let (failed, body) = mcp("wake_pc", json!({ "name": "game" })).await;
    assert!(failed, "{body}");
    assert_eq!(body["error"], "pc_not_connected", "{body}");
}

#[tokio::test]
async fn reaches_a_pc_whose_name_needs_escaping_in_the_path() {
    let addr = serve().await;
    let mut pc = join(addr, json!({ "type": "hello", "name": "書斎の PC/2" })).await;

    let press = tokio::spawn(call(
        addr,
        "POST",
        "/api/pcs/%E6%9B%B8%E6%96%8E%E3%81%AE%20PC%2F2/sleep",
    ));
    let command = next_text(&mut pc).await;
    pc.send(Message::text(
        json!({ "type": "result", "id": command["id"], "ok": true }).to_string(),
    ))
    .await
    .unwrap();
    let (status, body) = press.await.unwrap();
    assert_eq!(status, 200, "{body}");
}
