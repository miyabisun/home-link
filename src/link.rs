//! The one way home-link talks to matterjs-server's WebSocket API.
//!
//! Every step, opening the connection, sending, waiting for a reply and closing, ends by a
//! deadline, so a matterjs-server that takes the connection and never answers holds up no
//! request and no loop. Every reply to a request to a device is read by [`Outcome::of`].

use std::{fmt, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout, timeout_at};
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Opening a connection: the TCP connect and the WebSocket upgrade, on the same host.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(3);
/// The `get_nodes` reply, every node with its attributes.
pub const NODES_TIMEOUT: Duration = Duration::from_secs(5);
/// The close handshake; the connection is dropped after it either way.
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
/// The most one request from the app or the admin screen spends on matterjs-server, every
/// connection of it together: under the 20 s the app waits for an answer once connected.
pub const REQUEST_BUDGET: Duration = Duration::from_secs(15);

/// The deadline of a request starting now.
#[must_use]
pub fn budget() -> Instant {
    Instant::now() + REQUEST_BUDGET
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Why a connection to matterjs-server could not carry a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Not opened: refused, not resolved, or the upgrade not answered in time.
    Unreachable(String),
    /// Opened, but this was not done by its deadline.
    Timeout(&'static str),
    /// Closed or broken, or it carried something that is not JSON.
    Dropped(String),
    /// matterjs-server answered `get_nodes` with an error.
    Failed(String),
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(error) => write!(f, "not connected: {error}"),
            Self::Timeout(what) => write!(f, "timed out: {what}"),
            Self::Dropped(error) => write!(f, "connection dropped: {error}"),
            Self::Failed(error) => write!(f, "matterjs-server failed: {error}"),
        }
    }
}

/// One connection to matterjs-server.
pub struct Link {
    ws: Socket,
}

impl Link {
    /// Opens a connection to `url` within [`OPEN_TIMEOUT`] and `deadline`.
    ///
    /// # Errors
    /// [`Fault::Unreachable`] when it is refused, not resolved or not upgraded in time.
    pub async fn open(url: &str, deadline: Instant) -> Result<Self, Fault> {
        let by = deadline.min(Instant::now() + OPEN_TIMEOUT);
        match timeout_at(by, connect_async(url)).await {
            Err(_) => Err(Fault::Unreachable("timed out opening".into())),
            Ok(Err(error)) => Err(Fault::Unreachable(error.to_string())),
            Ok(Ok((ws, _))) => Ok(Self { ws }),
        }
    }

    /// The server info matterjs-server greets every connection with.
    ///
    /// # Errors
    /// When none comes within [`OPEN_TIMEOUT`] and `deadline`, or the connection drops.
    pub async fn info(&mut self, deadline: Instant) -> Result<Value, Fault> {
        let by = deadline.min(Instant::now() + OPEN_TIMEOUT);
        self.next(by).await?.ok_or(Fault::Timeout("server info"))
    }

    /// Sends `request` as it is.
    ///
    /// # Errors
    /// When it is not sent by `deadline` or the connection drops.
    pub async fn send(&mut self, request: &Value, deadline: Instant) -> Result<(), Fault> {
        match timeout_at(deadline, self.ws.send(Message::text(request.to_string()))).await {
            Err(_) => Err(Fault::Timeout("sending")),
            Ok(sent) => sent.map_err(|error| Fault::Dropped(error.to_string())),
        }
    }

    /// The next reply or event; `None` once `deadline` passes.
    ///
    /// # Errors
    /// When the connection drops or carries something that is not JSON.
    pub async fn next(&mut self, deadline: Instant) -> Result<Option<Value>, Fault> {
        loop {
            let Ok(message) = timeout_at(deadline, self.ws.next()).await else {
                return Ok(None);
            };
            match message {
                None => return Err(Fault::Dropped("closed before the reply".into())),
                Some(Err(error)) => return Err(Fault::Dropped(error.to_string())),
                Some(Ok(Message::Text(text))) => {
                    return serde_json::from_str(&text)
                        .map(Some)
                        .map_err(|error| Fault::Dropped(error.to_string()));
                }
                Some(Ok(_)) => {}
            }
        }
    }

    /// Sends `request` under a fresh message id and waits for its reply until `deadline`,
    /// skipping other messages; `None` without one.
    ///
    /// # Errors
    /// As [`Self::send`] and [`Self::next`].
    pub async fn request(
        &mut self,
        mut request: Value,
        deadline: Instant,
    ) -> Result<Option<Value>, Fault> {
        let id = next_id();
        request["message_id"] = json!(id);
        self.send(&request, deadline).await?;
        while let Some(reply) = self.next(deadline).await? {
            if reply["message_id"] == id.as_str() {
                return Ok(Some(reply));
            }
        }
        Ok(None)
    }

    /// Sends every request at once, each under a fresh message id, and returns each one's
    /// reply in order, `None` for those unanswered by `deadline`.
    ///
    /// # Errors
    /// As [`Self::send`] and [`Self::next`].
    pub async fn exchange<K>(
        &mut self,
        requests: Vec<(K, Value)>,
        deadline: Instant,
    ) -> Result<Vec<(K, Option<Value>)>, Fault> {
        let mut ids = Vec::with_capacity(requests.len());
        let mut replies = Vec::with_capacity(requests.len());
        for (key, mut request) in requests {
            let id = next_id();
            request["message_id"] = json!(id);
            self.send(&request, deadline).await?;
            ids.push(id);
            replies.push((key, None));
        }
        let mut unanswered = replies.len();
        while unanswered > 0 {
            let Some(reply) = self.next(deadline).await? else {
                break;
            };
            if let Some(i) = ids.iter().position(|id| reply["message_id"] == id.as_str())
                && replies[i].1.is_none()
            {
                replies[i].1 = Some(reply);
                unanswered -= 1;
            }
        }
        Ok(replies)
    }

    /// Every node, read within [`NODES_TIMEOUT`] and `deadline`.
    ///
    /// # Errors
    /// When they do not come in time, matterjs-server fails them or the connection drops.
    pub async fn nodes(&mut self, deadline: Instant) -> Result<Vec<Value>, Fault> {
        let by = deadline.min(Instant::now() + NODES_TIMEOUT);
        let reply = self
            .request(json!({ "command": "get_nodes" }), by)
            .await?
            .ok_or(Fault::Timeout("get_nodes"))?;
        nodes_of(reply)
    }

    /// Closes the connection, waiting at most [`CLOSE_TIMEOUT`] for matterjs-server.
    pub async fn close(mut self) {
        let _ = timeout(CLOSE_TIMEOUT, self.ws.close(None)).await;
    }
}

/// The nodes a `get_nodes` reply carries.
///
/// # Errors
/// [`Fault::Failed`] when matterjs-server failed the request.
pub fn nodes_of(mut reply: Value) -> Result<Vec<Value>, Fault> {
    match reply["result"].take() {
        Value::Array(nodes) => Ok(nodes),
        _ => Err(Fault::Failed(
            error(&reply).unwrap_or_else(|| "no nodes".into()),
        )),
    }
}

/// A message id no other request on any connection carries.
#[must_use]
pub fn next_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "request-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// matterjs-server's details of an error reply; `None` for a reply that is not one.
#[must_use]
pub fn error(reply: &Value) -> Option<String> {
    reply.get("error_code").map(|_| {
        reply["details"]
            .as_str()
            .unwrap_or("unknown error")
            .to_owned()
    })
}

/// What became of one request to a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The device, or the bridge in front of it, accepted it.
    Accepted,
    /// Not sent: the node is unavailable, or its bridge does not reach the device.
    NotSent,
    /// Sent, but not answered in time.
    NoAnswer,
    /// matterjs-server failed it itself, such as for a node it is not connected to, with its details.
    ServerError(String),
    /// The device refused it with an Interaction Model status, with matterjs-server's details.
    Refused(String),
    /// Nothing sent: the ledger ignores the device, or an override holds it.
    Ignored,
}

impl Outcome {
    /// What the reply to a request to a device says, `None` when none came in time.
    ///
    /// An error reply carrying a `status` comes from the device; one without is
    /// matterjs-server's own. A write answers with each attribute's status, and any but 0
    /// refused it.
    #[must_use]
    pub fn of(reply: Option<&Value>) -> Self {
        let Some(reply) = reply else {
            return Self::NoAnswer;
        };
        if let Some(details) = error(reply) {
            return match reply.get("status") {
                Some(status) => Self::Refused(format!("{details} (status {status})")),
                None => Self::ServerError(details),
            };
        }
        let refused = reply["result"].as_array().and_then(|statuses| {
            statuses
                .iter()
                .find_map(|s| s["Status"].as_u64().filter(|status| *status != 0))
        });
        refused.map_or(Self::Accepted, |status| {
            Self::Refused(format!("attribute status {status}"))
        })
    }

    /// The result as the API has always named it: `switched`, `no_response`, `failed` or `ignored`.
    #[must_use]
    pub const fn result(&self) -> &'static str {
        match self {
            Self::Accepted => "switched",
            Self::NotSent | Self::NoAnswer => "no_response",
            Self::ServerError(_) | Self::Refused(_) => "failed",
            Self::Ignored => "ignored",
        }
    }

    /// Why it was not accepted: `not_connected`, `no_answer`, `server_error` or `refused`.
    #[must_use]
    pub const fn reason(&self) -> Option<&'static str> {
        match self {
            Self::NotSent => Some("not_connected"),
            Self::NoAnswer => Some("no_answer"),
            Self::ServerError(_) => Some("server_error"),
            Self::Refused(_) => Some("refused"),
            Self::Accepted | Self::Ignored => None,
        }
    }

    /// matterjs-server's details of an error.
    #[must_use]
    pub fn details(&self) -> Option<&str> {
        match self {
            Self::ServerError(details) | Self::Refused(details) => Some(details),
            _ => None,
        }
    }

    /// Logs on one warning line what became of a request `what` sent, or meant to send,
    /// to `endpoint` of `node_id`, unless accepted or ignored.
    pub fn warn(&self, what: &str, node_id: u64, endpoint: u16) {
        if let Some(reason) = self.reason() {
            tracing::warn!(
                what,
                node_id,
                endpoint,
                reason,
                details = self.details().unwrap_or_default(),
                "device missed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Outcome;

    #[test]
    fn replies_tell_the_server_from_the_device() {
        assert_eq!(Outcome::of(None), Outcome::NoAnswer);
        assert_eq!(
            Outcome::of(Some(&json!({ "message_id": "1", "result": null }))),
            Outcome::Accepted
        );
        // matterjs-server 1.5.2 words an error of its own without a status.
        let unreached =
            json!({ "message_id": "1", "error_code": 0, "details": "Node 17 is not connected" });
        assert_eq!(
            Outcome::of(Some(&unreached)),
            Outcome::ServerError("Node 17 is not connected".into())
        );
        // A StatusResponse from the device carries its status, and a cluster's own when there is one.
        let refused = json!({ "message_id": "1", "error_code": 0, "details": "Unsupported command", "status": 129 });
        assert_eq!(
            Outcome::of(Some(&refused)),
            Outcome::Refused("Unsupported command (status 129)".into())
        );
        let write = json!({ "message_id": "1", "result": [{ "Path": {}, "Status": 0 }, { "Path": {}, "Status": 135 }] });
        assert_eq!(
            Outcome::of(Some(&write)),
            Outcome::Refused("attribute status 135".into())
        );
        let written = json!({ "message_id": "1", "result": [{ "Path": {}, "Status": 0 }] });
        assert_eq!(Outcome::of(Some(&written)), Outcome::Accepted);
    }
}
