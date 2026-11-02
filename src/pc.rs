//! PCs that keep a WebSocket open to home-link (`GET /api/pcs/connect`), and the sleep and
//! wake sent down it. A PC opens the socket itself, so it needs no inbound port; home-link
//! knows only the PCs connected now, kept in memory by the name each says hello with.
//!
//! The PC speaks first, then either side sends JSON text messages:
//! - PC: `{"type":"hello","name":…,"sleeping":bool?}` once, `{"type":"state","sleeping":bool?}`
//!   whenever it changes, and `{"type":"result","id":…,"ok":bool,"message":…?,"sleeping":bool?}`
//!   for each command.
//! - home-link: `{"type":"command","id":…,"action":"sleep"|"wake"}`, and
//!   `{"type":"error","message":…}` before closing a socket without a valid hello.

use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Json,
    extract::{
        ConnectInfo, Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::{ApiError, ApiResult, Db, UNPOISONED, clean_name};

/// How long each step with a PC may take.
#[derive(Clone, Copy, Debug)]
pub struct Times {
    /// From opening the socket to the hello.
    pub hello: Duration,
    /// Between the pings home-link sends.
    pub ping: Duration,
    /// Without any message, pongs included, before the PC counts as gone.
    pub idle: Duration,
    /// For the result of one command.
    pub answer: Duration,
}

impl Default for Times {
    fn default() -> Self {
        Self {
            hello: Duration::from_secs(10),
            ping: Duration::from_secs(20),
            idle: Duration::from_mins(1),
            answer: Duration::from_secs(10),
        }
    }
}

/// What a PC is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Sleep,
    Wake,
}

/// A message from a PC.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Incoming {
    Hello {
        name: String,
        #[serde(default)]
        sleeping: Option<bool>,
    },
    State {
        #[serde(default)]
        sleeping: Option<bool>,
    },
    Result {
        id: u64,
        ok: bool,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        sleeping: Option<bool>,
    },
}

/// A PC's result for one command.
struct Answer {
    ok: bool,
    message: Option<String>,
}

/// One connected PC.
struct Peer {
    /// Tells this socket from the one that replaced it under the same name.
    socket: u64,
    sleeping: Option<bool>,
    outbox: mpsc::UnboundedSender<String>,
    /// The commands sent and not answered yet, by id; dropped with the socket.
    waiting: HashMap<u64, oneshot::Sender<Answer>>,
}

/// A PC as `GET /api/pcs` shows it.
#[derive(Debug, PartialEq, Serialize)]
pub struct View {
    pub name: String,
    pub sleeping: Option<bool>,
}

/// The PCs connected now, by name.
#[derive(Default)]
pub(crate) struct Pcs {
    peers: Mutex<BTreeMap<String, Peer>>,
    next: AtomicU64,
}

/// Why a command did not go through.
#[derive(Debug, PartialEq)]
enum Failure {
    NotConnected,
    Rejected(Option<String>),
    NoAnswer,
    Disconnected,
}

impl Pcs {
    fn list(&self) -> Vec<View> {
        self.peers
            .lock()
            .expect(UNPOISONED)
            .iter()
            .map(|(name, peer)| View {
                name: name.clone(),
                sleeping: peer.sleeping,
            })
            .collect()
    }

    /// Takes `name`'s socket, closing the one it had before; returns the socket's number.
    fn join(
        &self,
        name: &str,
        sleeping: Option<bool>,
        outbox: mpsc::UnboundedSender<String>,
    ) -> u64 {
        let socket = self.next.fetch_add(1, Ordering::Relaxed);
        let peer = Peer {
            socket,
            sleeping,
            outbox,
            waiting: HashMap::new(),
        };
        let old = self
            .peers
            .lock()
            .expect(UNPOISONED)
            .insert(name.to_owned(), peer);
        if old.is_some() {
            tracing::info!(pc = name, "PC connected again; the old socket is closed");
        }
        socket
    }

    /// Forgets `name` unless another socket has taken its place.
    fn leave(&self, name: &str, socket: u64) {
        let mut peers = self.peers.lock().expect(UNPOISONED);
        if peers.get(name).is_some_and(|peer| peer.socket == socket) {
            peers.remove(name);
        }
    }

    /// Runs `change` on `name`'s peer while `socket` is still its socket.
    fn with_peer(&self, name: &str, socket: u64, change: impl FnOnce(&mut Peer)) {
        if let Some(peer) = self
            .peers
            .lock()
            .expect(UNPOISONED)
            .get_mut(name)
            .filter(|peer| peer.socket == socket)
        {
            change(peer);
        }
    }

    /// Takes one message from `name`'s socket.
    fn receive(&self, name: &str, socket: u64, message: Incoming) {
        self.with_peer(name, socket, |peer| match message {
            Incoming::Hello { .. } => {}
            Incoming::State { sleeping } => peer.sleeping = sleeping.or(peer.sleeping),
            Incoming::Result {
                id,
                ok,
                message,
                sleeping,
            } => {
                peer.sleeping = sleeping.or(peer.sleeping);
                if let Some(waiter) = peer.waiting.remove(&id) {
                    // The caller may have stopped waiting.
                    let _ = waiter.send(Answer { ok, message });
                }
            }
        });
    }

    /// Sends `action` to `name` and waits up to `answer` for its result; returns the PC's
    /// state afterwards.
    async fn command(
        &self,
        name: &str,
        action: Action,
        answer: Duration,
    ) -> Result<Option<bool>, Failure> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (reply, result) = oneshot::channel();
        let socket = {
            let mut peers = self.peers.lock().expect(UNPOISONED);
            let peer = peers.get_mut(name).ok_or(Failure::NotConnected)?;
            let message = json!({ "type": "command", "id": id, "action": action });
            peer.outbox
                .send(message.to_string())
                .map_err(|_| Failure::Disconnected)?;
            peer.waiting.insert(id, reply);
            peer.socket
        };
        match tokio::time::timeout(answer, result).await {
            Ok(Ok(Answer { ok: true, .. })) => Ok(self
                .peers
                .lock()
                .expect(UNPOISONED)
                .get(name)
                .and_then(|peer| peer.sleeping)),
            Ok(Ok(Answer { message, .. })) => Err(Failure::Rejected(message)),
            Ok(Err(_)) => Err(Failure::Disconnected),
            Err(_) => {
                self.with_peer(name, socket, |peer| {
                    peer.waiting.remove(&id);
                });
                Err(Failure::NoAnswer)
            }
        }
    }
}

/// Reads a hello, or says why the text is not one.
fn hello(text: &str) -> Result<(String, Option<bool>), &'static str> {
    match serde_json::from_str::<Incoming>(text) {
        Ok(Incoming::Hello { name, sleeping }) => clean_name(&name, false)
            .map(|name| (name, sleeping))
            .ok_or("名前は1〜100文字にしてください"),
        _ => Err("最初にhelloを送ってください"),
    }
}

pub(crate) async fn list(State(state): State<Db>) -> Json<Vec<View>> {
    Json(state.pcs.list())
}

pub(crate) async fn connect(
    State(state): State<Db>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| serve(state, socket, peer))
}

/// Keeps one PC's socket until it closes, stops answering or is replaced.
async fn serve(state: Db, socket: WebSocket, address: SocketAddr) {
    let times = state.pc_times;
    let (mut sink, mut stream) = socket.split();
    let first = tokio::time::timeout(times.hello, async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(text))) => return Some(text),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return None,
            }
        }
    })
    .await;
    let Ok(Some(text)) = first else {
        tracing::info!(peer = %address.ip(), "PC socket closed without a hello");
        let _ = sink.close().await;
        return;
    };
    let (name, sleeping) = match hello(&text) {
        Ok(hello) => hello,
        Err(message) => {
            tracing::warn!(peer = %address.ip(), message, "PC socket refused");
            let error = json!({ "type": "error", "message": message });
            let _ = sink.send(Message::text(error.to_string())).await;
            let _ = sink.close().await;
            return;
        }
    };

    let (outbox, mut queue) = mpsc::unbounded_channel::<String>();
    let socket = state.pcs.join(&name, sleeping, outbox);
    tracing::info!(pc = name, peer = %address.ip(), "PC connected");
    // Writes the commands and the pings until the peer is forgotten or replaced.
    let writer = tokio::spawn(async move {
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + times.ping, times.ping);
        loop {
            let message = tokio::select! {
                text = queue.recv() => match text {
                    Some(text) => Message::text(text),
                    None => break,
                },
                _ = ping.tick() => Message::Ping(axum::body::Bytes::new()),
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let reason = loop {
        match tokio::time::timeout(times.idle, stream.next()).await {
            Err(_) => break "no answer",
            Ok(None | Some(Err(_) | Ok(Message::Close(_)))) => break "closed",
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(&text) {
                Ok(message) => state.pcs.receive(&name, socket, message),
                Err(error) => tracing::debug!(pc = name, %error, "PC message ignored"),
            },
            Ok(Some(Ok(_))) => {}
        }
    };
    state.pcs.leave(&name, socket);
    writer.abort();
    tracing::info!(pc = name, reason, "PC disconnected");
}

pub(crate) async fn sleep(state: State<Db>, name: Path<String>) -> ApiResult<Json<Value>> {
    command(state, name, Action::Sleep).await
}

pub(crate) async fn wake(state: State<Db>, name: Path<String>) -> ApiResult<Json<Value>> {
    command(state, name, Action::Wake).await
}

async fn command(
    State(state): State<Db>,
    Path(name): Path<String>,
    action: Action,
) -> ApiResult<Json<Value>> {
    let verb = match action {
        Action::Sleep => "スリープ",
        Action::Wake => "復帰",
    };
    match state
        .pcs
        .command(&name, action, state.pc_times.answer)
        .await
    {
        Ok(sleeping) => {
            tracing::info!(pc = name, ?action, "PC command accepted");
            Ok(Json(json!({ "status": "accepted", "sleeping": sleeping })))
        }
        Err(failure) => {
            tracing::warn!(pc = name, ?action, ?failure, "PC command failed");
            Err(match failure {
                Failure::NotConnected => ApiError(
                    StatusCode::NOT_FOUND,
                    "pc_not_connected",
                    format!("{name}はhome-linkにつながっていません"),
                ),
                Failure::Rejected(message) => ApiError(
                    StatusCode::BAD_GATEWAY,
                    "pc_rejected",
                    match message {
                        Some(message) => format!("{name}が{verb}を受け付けませんでした: {message}"),
                        None => format!("{name}が{verb}を受け付けませんでした"),
                    },
                ),
                Failure::NoAnswer => ApiError(
                    StatusCode::GATEWAY_TIMEOUT,
                    "pc_no_answer",
                    format!("{name}が{verb}に答えませんでした"),
                ),
                Failure::Disconnected => ApiError(
                    StatusCode::BAD_GATEWAY,
                    "pc_disconnected",
                    format!("{name}への接続が{verb}の途中で切れました"),
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hello_needs_a_name_of_one_to_a_hundred_characters() {
        assert_eq!(
            hello(r#"{"type":"hello","name":" work ","sleeping":true}"#),
            Ok(("work".to_owned(), Some(true)))
        );
        assert_eq!(
            hello(r#"{"type":"hello","name":"work"}"#),
            Ok(("work".to_owned(), None))
        );
        assert!(
            hello(r#"{"type":"hello","name":" "}"#).is_err(),
            "a blank name"
        );
        let long = format!(r#"{{"type":"hello","name":"{}"}}"#, "あ".repeat(101));
        assert!(hello(&long).is_err(), "101 characters");
        assert!(hello(r#"{"type":"state"}"#).is_err(), "not a hello");
        assert!(hello("work").is_err(), "not JSON");
    }

    #[test]
    fn a_result_without_a_state_keeps_the_last_one() {
        let pcs = Pcs::default();
        let (outbox, _queue) = mpsc::unbounded_channel();
        let socket = pcs.join("work", Some(true), outbox);
        pcs.receive(
            "work",
            socket,
            Incoming::Result {
                id: 9,
                ok: true,
                message: None,
                sleeping: None,
            },
        );
        pcs.receive("work", socket, Incoming::State { sleeping: None });
        assert_eq!(
            pcs.list(),
            [View {
                name: "work".into(),
                sleeping: Some(true)
            }]
        );
    }

    #[test]
    fn an_old_socket_neither_changes_nor_removes_its_replacement() {
        let pcs = Pcs::default();
        let (old_outbox, _old) = mpsc::unbounded_channel();
        let (new_outbox, _new) = mpsc::unbounded_channel();
        let old = pcs.join("work", Some(false), old_outbox);
        let new = pcs.join("work", Some(true), new_outbox);
        assert_ne!(old, new, "each socket has its own number");

        pcs.receive(
            "work",
            old,
            Incoming::State {
                sleeping: Some(false),
            },
        );
        pcs.leave("work", old);
        assert_eq!(
            pcs.list(),
            [View {
                name: "work".into(),
                sleeping: Some(true)
            }]
        );
        pcs.leave("work", new);
        assert_eq!(pcs.list(), []);
    }
}
