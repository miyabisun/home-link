use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Basic Information (0x0028) on a node, Bridged Device Basic Information
/// (0x0039) on each device behind a bridge.
const IDENTITY_CLUSTERS: [&str; 2] = ["40", "57"];
const VENDOR_NAME: &str = "1";
const SERIAL_NUMBER: &str = "15";

/// Finds the node and endpoint whose identity cluster carries `vendor` and
/// `serial` in a matterjs-server `get_nodes` result.
#[must_use]
pub fn locate(nodes: &[Value], vendor: &str, serial: &str) -> Option<(u64, u16)> {
    nodes.iter().find_map(|node| {
        let attributes = node["attributes"].as_object()?;
        let endpoint = attributes.iter().find_map(|(path, value)| {
            let mut parts = path.split('/');
            let (endpoint, cluster) = (parts.next()?, parts.next()?);
            let identity = IDENTITY_CLUSTERS.contains(&cluster)
                && parts.next() == Some(SERIAL_NUMBER)
                && value == serial
                && attributes.get(&format!("{endpoint}/{cluster}/{VENDOR_NAME}"))? == vendor;
            identity.then(|| endpoint.parse().ok()).flatten()
        })?;
        Some((node["node_id"].as_u64()?, endpoint))
    })
}

/// On/Off, Dimmable, Color Temperature and Extended Color Light device types.
const LIGHT_TYPES: [u64; 4] = [0x0100, 0x0101, 0x010C, 0x010D];

/// A light endpoint served by matterjs-server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Light {
    pub node_id: u64,
    pub endpoint: u16,
    /// Behind a bridge, identified by Bridged Device Basic Information on its own endpoint.
    pub bridged: bool,
    /// The node is available and, behind a bridge, the bridge reaches the device.
    pub reachable: bool,
    /// The On/Off attribute as matterjs-server last read it.
    pub on: Option<bool>,
    pub product: Option<String>,
}

/// Lists every endpoint, including those behind bridges, whose Descriptor
/// device type list holds a light type and which carries On/Off.
#[must_use]
pub fn lights(nodes: &[Value]) -> Vec<Light> {
    let mut lights = Vec::new();
    for node in nodes {
        let (Some(node_id), Some(attributes)) =
            (node["node_id"].as_u64(), node["attributes"].as_object())
        else {
            continue;
        };
        for (path, types) in attributes {
            let Some(endpoint) = path
                .strip_suffix("/29/0")
                .and_then(|ep| ep.parse::<u16>().ok())
            else {
                continue;
            };
            let is_light = types.as_array().is_some_and(|types| {
                types
                    .iter()
                    .any(|t| t["0"].as_u64().is_some_and(|t| LIGHT_TYPES.contains(&t)))
            });
            let Some(on) = attributes.get(&format!("{endpoint}/6/0")) else {
                continue;
            };
            if !is_light {
                continue;
            }
            let bridged = attributes.contains_key(&format!("{endpoint}/57/3"))
                || attributes.contains_key(&format!("{endpoint}/57/17"));
            let reachable = node["available"] != false
                && attributes.get(&format!("{endpoint}/57/17")) != Some(&Value::Bool(false));
            let product = if bridged {
                format!("{endpoint}/57/3")
            } else {
                "0/40/3".to_owned()
            };
            lights.push(Light {
                node_id,
                endpoint,
                bridged,
                reachable,
                on: on.as_bool(),
                product: attributes
                    .get(&product)
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
    }
    lights.sort_by_key(|light| (light.node_id, light.endpoint));
    lights
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Reads every node from the matterjs-server WebSocket API at `url`.
///
/// # Errors
/// Fails when the server cannot be reached, times out, or answers with an error.
pub async fn fetch_nodes(url: &str) -> Result<Vec<Value>, String> {
    tokio::time::timeout(TIMEOUT, async {
        let mut ws = connect(url).await?;
        let nodes = get_nodes(&mut ws).await.map_err(|e| e.0);
        let _ = ws.close(None).await;
        nodes
    })
    .await
    .map_err(|_| "timed out".to_owned())?
}

/// What became of the command to one light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The device, or the bridge in front of it, accepted the command.
    Switched,
    /// Unavailable, unreachable behind its bridge, or no answer in time.
    NoResponse,
    /// matterjs-server answered with an error.
    Failed,
}

/// Sends On or Off to every reachable light and reports each light's outcome,
/// with the nodes read in the same session.
///
/// A connection dropped mid-session is opened again and the whole switch
/// repeated (On and Off are absolute) until the deadline; a server that refuses
/// the first connection fails at once.
///
/// # Errors
/// Fails when the server cannot be reached or its nodes cannot be read in time.
pub async fn switch(url: &str, on: bool) -> Result<(Vec<Value>, Vec<(Light, Outcome)>), String> {
    let deadline = Instant::now() + TIMEOUT;
    let mut ws = connect(url).await?;
    loop {
        let result = tokio::time::timeout_at(deadline, switch_once(&mut ws, on, deadline)).await;
        let _ = ws.close(None).await;
        let Dropped(error) = match result {
            Err(_) => return Err("timed out".into()),
            Ok(Ok(done)) => return Ok(done),
            Ok(Err(dropped)) => dropped,
        };
        tracing::warn!(%error, "matterjs-server connection dropped; reconnecting");
        ws = loop {
            tokio::time::sleep(RECONNECT_DELAY).await;
            if Instant::now() >= deadline {
                return Err(error);
            }
            if let Ok(ws) = connect(url).await {
                break ws;
            }
        };
    }
}

const RECONNECT_DELAY: Duration = Duration::from_millis(500);

/// The connection ended or failed before the expected replies arrived.
struct Dropped(String);

async fn switch_once(
    ws: &mut Socket,
    on: bool,
    deadline: Instant,
) -> Result<(Vec<Value>, Vec<(Light, Outcome)>), Dropped> {
    let nodes = get_nodes(ws).await?;
    let mut results: Vec<_> = lights(&nodes)
        .into_iter()
        .map(|light| (light, Outcome::NoResponse))
        .collect();
    let command = if on { "On" } else { "Off" };
    let mut pending = 0;
    for (index, (light, _)) in results.iter().enumerate() {
        if !light.reachable {
            continue;
        }
        let request = json!({
            "message_id": index.to_string(),
            "command": "device_command",
            "args": {
                "node_id": light.node_id,
                "endpoint_id": light.endpoint,
                "cluster_id": 6,
                "command_name": command,
                "payload": {},
            },
        });
        send(ws, &request).await?;
        pending += 1;
    }
    while pending > 0 {
        let Ok(reply) = tokio::time::timeout_at(deadline, receive(ws)).await else {
            break; // Lights still unanswered stay NoResponse.
        };
        let reply = reply?;
        let Some(index) = reply["message_id"]
            .as_str()
            .and_then(|id| id.parse::<usize>().ok())
        else {
            continue;
        };
        if let Some((_, outcome)) = results.get_mut(index) {
            *outcome = if reply.get("error_code").is_some() {
                Outcome::Failed
            } else {
                Outcome::Switched
            };
            pending -= 1;
        }
    }
    Ok((nodes, results))
}

async fn connect(url: &str) -> Result<Socket, String> {
    let (ws, _) = connect_async(url).await.map_err(|e| e.to_string())?;
    Ok(ws)
}

async fn send(ws: &mut Socket, request: &Value) -> Result<(), Dropped> {
    ws.send(Message::text(request.to_string()))
        .await
        .map_err(|e| Dropped(e.to_string()))
}

/// The next reply or event; JSON that does not parse is a protocol failure.
async fn receive(ws: &mut Socket) -> Result<Value, Dropped> {
    while let Some(message) = ws.next().await {
        if let Message::Text(text) = message.map_err(|e| Dropped(e.to_string()))? {
            return serde_json::from_str(&text).map_err(|e| Dropped(e.to_string()));
        }
    }
    Err(Dropped("connection closed before the reply".into()))
}

async fn get_nodes(ws: &mut Socket) -> Result<Vec<Value>, Dropped> {
    send(
        ws,
        &json!({ "message_id": "get_nodes", "command": "get_nodes" }),
    )
    .await?;
    loop {
        let mut reply = receive(ws).await?;
        if reply["message_id"] != "get_nodes" {
            continue;
        }
        return match reply["result"].take() {
            Value::Array(nodes) => Ok(nodes),
            _ => Err(Dropped(format!(
                "get_nodes failed: {}",
                reply["error_code"]
            ))),
        };
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Light, lights, locate};

    // Trimmed from a matterjs-server `get_nodes` result (2026-10-05).
    fn nodes() -> Vec<serde_json::Value> {
        vec![
            json!({ "node_id": 1, "attributes": {
                "0/40/1": "Aqara", "0/40/15": "54ef4475b119",
                "2/57/1": "Aqara", "2/57/15": "54ef44100126e0a6",
                "3/57/1": "Aqara", "3/57/15": "54ef441001372c4c",
            }}),
            json!({ "node_id": 5, "attributes": { "0/40/1": "Tapo", "0/40/15": "CCBABDE0C244" }}),
            json!({ "node_id": 16, "attributes": {
                "0/40/1": "Uascent", "0/40/15": "U20251209000001453",
            }}),
        ]
    }

    #[test]
    fn locates_root_devices_by_basic_information() {
        assert_eq!(locate(&nodes(), "Tapo", "CCBABDE0C244"), Some((5, 0)));
        assert_eq!(
            locate(&nodes(), "Uascent", "U20251209000001453"),
            Some((16, 0))
        );
    }

    #[test]
    fn locates_bridged_devices_by_their_endpoint() {
        assert_eq!(locate(&nodes(), "Aqara", "54ef441001372c4c"), Some((1, 3)));
        assert_eq!(locate(&nodes(), "Aqara", "54ef4475b119"), Some((1, 0)));
    }

    #[test]
    fn unseen_or_other_vendor_serials_are_not_located() {
        assert_eq!(locate(&nodes(), "Tapo", "CCBABDE0FFFF"), None);
        assert_eq!(locate(&nodes(), "Aqara", "CCBABDE0C244"), None);
        // A serial number on another cluster's attribute is not an identity.
        let other = vec![json!({ "node_id": 2, "attributes": { "1/6/15": "X", "1/6/1": "V" }})];
        assert_eq!(locate(&other, "V", "X"), None);
        assert_eq!(locate(&[json!({ "node_id": 3 })], "V", "X"), None);
    }

    fn light(
        node_id: u64,
        endpoint: u16,
        bridged: bool,
        reachable: bool,
        on: Option<bool>,
    ) -> Light {
        Light {
            node_id,
            endpoint,
            bridged,
            reachable,
            on,
            product: Some("Bulb".into()),
        }
    }

    #[test]
    fn lights_are_endpoints_with_a_light_device_type_and_on_off() {
        // Trimmed from a matterjs-server `get_nodes` result (2026-10-05).
        let nodes = vec![
            json!({ "node_id": 1, "available": true, "attributes": {
                "0/29/0": [{ "0": 18, "1": 1 }, { "0": 22, "1": 4 }], "0/40/3": "Hub",
                "1/29/0": [{ "0": 14, "1": 2 }],
                "2/29/0": [{ "0": 19, "1": 2 }, { "0": 268, "1": 4 }], "2/6/0": false,
                "2/57/3": "Bulb", "2/57/17": false,
                "3/29/0": [{ "0": 19, "1": 2 }, { "0": 268, "1": 4 }], "3/6/0": true,
                "3/57/3": "Bulb", "3/57/17": true,
            }}),
            json!({ "node_id": 5, "available": true, "attributes": {
                "0/29/0": [{ "0": 22, "1": 1 }], "0/40/3": "Bulb",
                "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": false,
            }}),
            json!({ "node_id": 16, "available": false, "attributes": {
                "0/40/3": "Bulb", "1/29/0": [{ "0": 256, "1": 1 }], "1/6/0": true,
            }}),
        ];
        assert_eq!(
            lights(&nodes),
            [
                light(1, 2, true, false, Some(false)),
                light(1, 3, true, true, Some(true)),
                light(5, 1, false, true, Some(false)),
                light(16, 1, false, false, Some(true)),
            ]
        );
    }

    #[test]
    fn plugs_switches_and_lights_without_on_off_are_not_lights() {
        let nodes = vec![
            // On/Off Plug-in Unit and On/Off Light Switch carry On/Off but are not lights.
            json!({ "node_id": 2, "attributes": {
                "1/29/0": [{ "0": 266, "1": 1 }], "1/6/0": true,
                "2/29/0": [{ "0": 259, "1": 1 }], "2/6/0": true,
            }}),
            json!({ "node_id": 3, "attributes": { "1/29/0": [{ "0": 257, "1": 1 }] }}),
            json!({ "node_id": 4, "attributes": { "1/6/0": true }}),
        ];
        assert_eq!(lights(&nodes), []);
        // Dimmable lights count; a missing state stays unknown and the product name optional.
        let dimmable = vec![json!({ "node_id": 6, "attributes": {
            "1/29/0": [{ "0": 257, "1": 1 }], "1/6/0": null,
        }})];
        assert_eq!(
            lights(&dimmable),
            [Light {
                product: None,
                ..light(6, 1, false, true, None)
            }]
        );
    }
}
