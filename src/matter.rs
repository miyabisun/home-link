use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::schedule::{Skip, Target};

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

/// Whether the node is available and, behind a bridge, the bridge reaches the device on `endpoint`.
fn reachable(node: &Value, endpoint: u16) -> bool {
    node["available"] != false
        && node["attributes"][format!("{endpoint}/57/17")] != Value::Bool(false)
}

/// Whether the device `locate` found answers, and its product name: Basic
/// Information on the root endpoint, Bridged Device Basic Information behind a bridge.
#[must_use]
pub fn describe(nodes: &[Value], node_id: u64, endpoint: u16) -> (bool, Option<String>) {
    let Some(node) = nodes.iter().find(|n| n["node_id"] == node_id) else {
        return (false, None);
    };
    let product = if endpoint == 0 {
        "0/40/3".to_owned()
    } else {
        format!("{endpoint}/57/3")
    };
    let name = node["attributes"][product].as_str().map(str::to_owned);
    (reachable(node, endpoint), name)
}

/// The network a node runs on by the feature map of its Network Commissioning (0x0031):
/// `wifi`, `thread` or `ethernet`; a device behind a bridge is on its bridge's.
#[must_use]
pub fn network(nodes: &[Value], node_id: u64) -> Option<&'static str> {
    let node = nodes.iter().find(|n| n["node_id"] == node_id)?;
    let features = node["attributes"]["0/49/65532"].as_u64()?;
    [(1, "wifi"), (2, "thread"), (4, "ethernet")]
        .into_iter()
        .find_map(|(bit, network)| (features & bit != 0).then_some(network))
}

/// On/Off, Dimmable, Color Temperature and Extended Color Light device types.
const LIGHT_TYPES: [u64; 4] = [0x0100, 0x0101, 0x010C, 0x010D];
/// On/Off and Dimmable Plug-in Unit device types.
const PLUG_TYPES: [u64; 2] = [0x010A, 0x010B];

/// The Room Air Conditioner device type.
const AIRCON_TYPES: [u64; 1] = [0x0072];

/// What a device is, by the Matter device types of its endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Light,
    Plug,
    /// Driven by the air conditioner's own control, not switched as a plug or with labels.
    Aircon,
    Other,
}

/// The endpoint carrying a device's On/Off, and the On/Off last read.
pub type OnOff = (u16, Option<bool>);

/// Whether `endpoint` is a device behind a bridge, by its Bridged Device Basic Information.
fn bridged(attributes: &serde_json::Map<String, Value>, endpoint: u16) -> bool {
    attributes.contains_key(&format!("{endpoint}/57/3"))
        || attributes.contains_key(&format!("{endpoint}/57/17"))
}

fn has_type(types: &Value, wanted: &[u64]) -> bool {
    types.as_array().is_some_and(|types| {
        types
            .iter()
            .any(|t| t["0"].as_u64().is_some_and(|t| wanted.contains(&t)))
    })
}

/// The kind of the device `locate` found and, unless it is a light, the endpoint
/// carrying its On/Off with the On/Off last read: its own endpoint behind a
/// bridge, else the first of its node's endpoints not behind a bridge.
// ponytail: one On/Off per device; a power strip's other outlets wait for a ledger row per endpoint.
#[must_use]
pub fn kind(nodes: &[Value], node_id: u64, endpoint: u16) -> Option<(Kind, Option<OnOff>)> {
    let node = nodes.iter().find(|n| n["node_id"] == node_id)?;
    let attributes = node["attributes"].as_object()?;
    let mut parts: Vec<(u16, &Value)> = attributes
        .iter()
        .filter_map(|(path, types)| {
            let part = path.strip_suffix("/29/0")?.parse().ok()?;
            let own = if endpoint == 0 {
                !bridged(attributes, part)
            } else {
                part == endpoint
            };
            own.then_some((part, types))
        })
        .collect();
    parts.sort_by_key(|(part, _)| *part);
    if parts.iter().any(|(_, types)| has_type(types, &LIGHT_TYPES)) {
        return Some((Kind::Light, None));
    }
    let kind = if parts
        .iter()
        .any(|(_, types)| has_type(types, &AIRCON_TYPES))
    {
        Kind::Aircon
    } else if parts.iter().any(|(_, types)| has_type(types, &PLUG_TYPES)) {
        Kind::Plug
    } else {
        Kind::Other
    };
    let on_off = parts.iter().find_map(|(part, _)| {
        let on = attributes.get(&format!("{part}/6/0"))?;
        Some((*part, on.as_bool()))
    });
    Some((kind, on_off))
}

/// The On/Off endpoint of the device `locate` found when it is not a light, such as a
/// plug, in the shape of a light that neither dims nor changes colour.
#[must_use]
pub fn outlet(nodes: &[Value], node_id: u64, endpoint: u16) -> Option<Light> {
    let (Kind::Plug | Kind::Other, Some((switch, on))) = kind(nodes, node_id, endpoint)? else {
        return None;
    };
    let node = nodes.iter().find(|n| n["node_id"] == node_id)?;
    let (reachable, product) = describe(nodes, node_id, endpoint);
    Some(Light {
        node_id,
        endpoint: switch,
        bridged: bridged(node["attributes"].as_object()?, switch),
        reachable,
        on,
        product,
        levels: None,
        mireds: None,
    })
}

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
    /// Min and max `CurrentLevel` when the light dims (Level Control).
    pub levels: Option<(u8, u8)>,
    /// Physical min and max colour temperature in mireds when the light
    /// supports it (Color Control with the `ColorTemperature` feature).
    pub mireds: Option<(u16, u16)>,
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
            let is_light = has_type(types, &LIGHT_TYPES);
            let Some(on) = attributes.get(&format!("{endpoint}/6/0")) else {
                continue;
            };
            if !is_light {
                continue;
            }
            let bridged = bridged(attributes, endpoint);
            let reachable = reachable(node, endpoint);
            let product = if bridged {
                format!("{endpoint}/57/3")
            } else {
                "0/40/3".to_owned()
            };
            let attribute = |path: String| attributes.get(&path).and_then(Value::as_u64);
            let levels = attributes
                .contains_key(&format!("{endpoint}/8/0"))
                .then(|| {
                    let min = attribute(format!("{endpoint}/8/2")).unwrap_or(1);
                    let max = attribute(format!("{endpoint}/8/3")).unwrap_or(254);
                    (narrow(min, 1), narrow(max, 254))
                });
            let mireds = attribute(format!("{endpoint}/768/65532"))
                .is_some_and(|features| features & COLOR_TEMPERATURE != 0)
                .then(|| {
                    attribute(format!("{endpoint}/768/16395"))
                        .zip(attribute(format!("{endpoint}/768/16396")))
                        .map_or((153, 370), |(min, max)| {
                            (narrow(min, 153), narrow(max, 370))
                        })
                });
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
                levels,
                mireds,
            });
        }
    }
    lights.sort_by_key(|light| (light.node_id, light.endpoint));
    lights
}

/// Color Control `FeatureMap` bit for colour temperature.
const COLOR_TEMPERATURE: u64 = 0x10;

fn narrow<T: TryFrom<u64>>(value: u64, fallback: T) -> T {
    T::try_from(value).unwrap_or(fallback)
}

pub(crate) type Socket =
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
    /// Ignored in the ledger: nothing sent.
    Ignored,
}

/// Sends On or Off to every reachable light but those `ignored` and reports each light's outcome,
/// with the nodes read in the same session.
///
/// All lights are sent their command at once, so a light that never answers holds up none of
/// the others. Those still unanswered after `RESEND_AFTER` are sent it again with the nodes
/// read again, and whatever has answered by `ANSWER_WINDOW` is reported: an answer to either
/// send counts. A connection dropped mid-session is opened again and the command sent again
/// to the lights yet to answer (On and Off are absolute) within the same window; a server that
/// refuses the first connection fails at once.
///
/// # Errors
/// Fails when the server cannot be reached or its nodes cannot be read in time.
pub async fn switch(
    url: &str,
    on: bool,
    ignored: impl Fn(&[Value], &Light) -> bool,
) -> Result<(Vec<Value>, Vec<(Light, Outcome)>), String> {
    switch_among(url, on, lights, ignored).await
}

/// `switch` over the endpoints `targets` picks from the nodes, lights or not.
///
/// # Errors
/// As `switch`.
pub async fn switch_among(
    url: &str,
    on: bool,
    targets: impl Fn(&[Value]) -> Vec<Light>,
    ignored: impl Fn(&[Value], &Light) -> bool,
) -> Result<(Vec<Value>, Vec<(Light, Outcome)>), String> {
    let mut ws = connect(url).await?;
    let nodes = tokio::time::timeout(TIMEOUT, get_nodes(&mut ws))
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.0)?;
    let mut switching = Switching::new(nodes, on, &targets, &ignored);
    let deadline = Instant::now() + ANSWER_WINDOW;
    let resend_at = Instant::now() + RESEND_AFTER;
    loop {
        let result = switching.run(&mut ws, resend_at, deadline, &targets).await;
        let _ = ws.close(None).await;
        let Err(Dropped(error)) = result else {
            break;
        };
        tracing::warn!(%error, "matterjs-server connection dropped; reconnecting");
        let Some(again) = reconnect(url, deadline).await else {
            break;
        };
        ws = again;
    }
    Ok((switching.nodes, switching.results))
}

/// A new connection to `url` before `deadline`, tried every `RECONNECT_DELAY`.
async fn reconnect(url: &str, deadline: Instant) -> Option<Socket> {
    loop {
        tokio::time::sleep(RECONNECT_DELAY).await;
        if Instant::now() >= deadline {
            return None;
        }
        if let Ok(Ok(ws)) = tokio::time::timeout_at(deadline, connect(url)).await {
            return Some(ws);
        }
    }
}

const RECONNECT_DELAY: Duration = Duration::from_millis(500);

/// The connection ended or failed before the expected replies arrived.
pub(crate) struct Dropped(pub(crate) String);

/// One switch across connections: each light's outcome so far and the sends awaiting an answer.
struct Switching {
    nodes: Vec<Value>,
    results: Vec<(Light, Outcome)>,
    command: &'static str,
    /// Message id of each send awaiting an answer on the current connection, and its light.
    awaiting: std::collections::HashMap<String, usize>,
    sends: Vec<usize>,
    resent: bool,
}

impl Switching {
    fn new(
        nodes: Vec<Value>,
        on: bool,
        targets: &impl Fn(&[Value]) -> Vec<Light>,
        ignored: &impl Fn(&[Value], &Light) -> bool,
    ) -> Self {
        let results: Vec<_> = targets(&nodes)
            .into_iter()
            .map(|light| {
                let outcome = if ignored(&nodes, &light) {
                    Outcome::Ignored
                } else {
                    Outcome::NoResponse
                };
                (light, outcome)
            })
            .collect();
        Self {
            sends: vec![0; results.len()],
            nodes,
            results,
            command: if on { "On" } else { "Off" },
            awaiting: std::collections::HashMap::new(),
            resent: false,
        }
    }

    fn settled(&self) -> bool {
        self.results.iter().all(|(_, o)| *o != Outcome::NoResponse)
    }

    /// Sends to every reachable light yet to answer that has been sent fewer than `ATTEMPTS`
    /// commands, then takes answers until all have answered or `deadline`, sending again
    /// once at `resend_at` with the nodes read again.
    async fn run(
        &mut self,
        ws: &mut Socket,
        resend_at: Instant,
        deadline: Instant,
        targets: &impl Fn(&[Value]) -> Vec<Light>,
    ) -> Result<(), Dropped> {
        // Sends on an earlier connection will not be answered on this one.
        self.awaiting.clear();
        self.send(ws).await?;
        while !self.settled() {
            let wait = if self.resent {
                deadline
            } else {
                resend_at.min(deadline)
            };
            match tokio::time::timeout_at(wait, receive(ws)).await {
                Ok(reply) => self.answer(&reply?),
                Err(_) if wait == deadline => break,
                Err(_) => {
                    self.resent = true;
                    if !self.reread(ws, deadline, targets).await? {
                        break;
                    }
                    self.send(ws).await?;
                }
            }
        }
        Ok(())
    }

    async fn send(&mut self, ws: &mut Socket) -> Result<(), Dropped> {
        for (index, (light, outcome)) in self.results.iter().enumerate() {
            if !light.reachable || *outcome != Outcome::NoResponse || self.sends[index] >= ATTEMPTS
            {
                continue;
            }
            let id = next_id();
            let request = json!({
                "message_id": id,
                "command": "device_command",
                "args": {
                    "node_id": light.node_id,
                    "endpoint_id": light.endpoint,
                    "cluster_id": 6,
                    "command_name": self.command,
                    "payload": {},
                },
            });
            send(ws, &request).await?;
            self.sends[index] += 1;
            self.awaiting.insert(id, index);
        }
        Ok(())
    }

    fn answer(&mut self, reply: &Value) {
        let Some(index) = reply["message_id"]
            .as_str()
            .and_then(|id| self.awaiting.remove(id))
        else {
            return;
        };
        let outcome = &mut self.results[index].1;
        if *outcome == Outcome::NoResponse {
            *outcome = if reply.get("error_code").is_some() {
                Outcome::Failed
            } else {
                Outcome::Switched
            };
        }
    }

    /// Reads the nodes again, taking answers that arrive meanwhile, and refreshes the lights
    /// yet to answer; `false` when the nodes do not come by `deadline`.
    async fn reread(
        &mut self,
        ws: &mut Socket,
        deadline: Instant,
        targets: &impl Fn(&[Value]) -> Vec<Light>,
    ) -> Result<bool, Dropped> {
        send(
            ws,
            &json!({ "message_id": "get_nodes", "command": "get_nodes" }),
        )
        .await?;
        let nodes = loop {
            let Ok(reply) = tokio::time::timeout_at(deadline, receive(ws)).await else {
                return Ok(false);
            };
            let mut reply = reply?;
            if reply["message_id"] != "get_nodes" {
                self.answer(&reply);
                continue;
            }
            match nodes_of(&mut reply) {
                Ok(nodes) => break nodes,
                Err(_) => return Ok(false),
            }
        };
        for light in targets(&nodes) {
            let key = (light.node_id, light.endpoint);
            if let Some((known, _)) = self
                .results
                .iter_mut()
                .find(|(l, o)| (l.node_id, l.endpoint) == key && *o == Outcome::NoResponse)
            {
                *known = light;
            }
        }
        self.nodes = nodes;
        Ok(true)
    }
}

/// At most two sends to a light that has not answered, the second after `RESEND_AFTER`; the
/// switch reports what has answered by `ANSWER_WINDOW` after the nodes are read.
const ATTEMPTS: usize = 2;
const RESEND_AFTER: Duration = Duration::from_secs(1);
const ANSWER_WINDOW: Duration = Duration::from_millis(2500);

/// Sends On or Off to one device's `endpoint`: `Some(true)` once accepted,
/// `Some(false)` on an error and `None` without an answer in time.
///
/// # Errors
/// Fails when the server cannot be reached or drops the connection.
pub async fn switch_one(
    url: &str,
    node_id: u64,
    endpoint: u16,
    on: bool,
) -> Result<Option<bool>, String> {
    let mut ws = connect(url).await?;
    let command = json!({
        "command": "device_command",
        "args": {
            "node_id": node_id,
            "endpoint_id": endpoint,
            "cluster_id": 6,
            "command_name": if on { "On" } else { "Off" },
            "payload": {},
        },
    });
    let reply = request(&mut ws, command).await.map_err(|e| e.0);
    let _ = ws.close(None).await;
    Ok(reply?.map(|reply| reply.get("error_code").is_none()))
}

/// Writes `steps` to `endpoint` of `node_id` in order, stopping at the first one refused or
/// unanswered: `Some(true)` once all are accepted, `Some(false)` on a refusal and `None`
/// without an answer in time.
///
/// # Errors
/// Fails when the server cannot be reached or drops the connection.
pub async fn drive(
    url: &str,
    node_id: u64,
    endpoint: u16,
    steps: &[crate::aircon::Step],
) -> Result<Option<bool>, String> {
    let mut ws = connect(url).await?;
    let result = async {
        for step in steps {
            let command = match *step {
                crate::aircon::Step::Write(path, value) => json!({
                    "command": "write_attribute",
                    "args": { "node_id": node_id, "attribute_path": format!("{endpoint}/{path}"), "value": value },
                }),
                crate::aircon::Step::Switch(on) => json!({
                    "command": "device_command",
                    "args": {
                        "node_id": node_id,
                        "endpoint_id": endpoint,
                        "cluster_id": 6,
                        "command_name": if on { "On" } else { "Off" },
                        "payload": {},
                    },
                }),
            };
            let Some(reply) = request(&mut ws, command).await? else {
                return Ok(None);
            };
            // A write answers with each attribute's status; anything but 0 refused it.
            let refused = reply.get("error_code").is_some()
                || reply["result"]
                    .as_array()
                    .is_some_and(|statuses| statuses.iter().any(|s| s["Status"].as_u64().is_some_and(|s| s != 0)));
            if refused {
                return Ok(Some(false));
            }
        }
        Ok(Some(true))
    }
    .await
    .map_err(|e: Dropped| e.0);
    let _ = ws.close(None).await;
    result
}

/// What became of one light in an adjustment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Left alone before its On/Off was read.
    Skipped(Skip),
    /// Read as off just before writing: nothing sent.
    Off,
    /// No answer to the read or to a command in time.
    NoResponse,
    /// matterjs-server answered a command with an error.
    Failed,
    Sent(Target),
}

/// An adjustment: the nodes read, each light's decision and the commands sent.
pub struct Tuned {
    pub nodes: Vec<Value>,
    pub lights: Vec<(Light, Decision)>,
    pub commands: usize,
}

const TUNE_TIMEOUT: Duration = Duration::from_mins(1);
const REPLY_TIMEOUT: Duration = Duration::from_secs(3);
/// Each scheduled change fades over 30 s (tenths of a second) so a step every ten minutes goes unnoticed.
pub const TRANSITION: u16 = 300;

/// Reads every light, asks `plan` what to send each given the nodes read, reads the On/Off of each
/// light to write to again, and sends the level and colour temperature only to
/// lights read as on, fading over `transition` tenths of a second. Commands
/// never carry `ExecuteIfOff`, so a light switched off in between ignores them
/// and stays off.
///
/// A light in `switched_on`, just switched on, is not read: its On may not be
/// reported yet. It gets the values at once, lighting it if still off.
///
/// # Errors
/// Fails when the server cannot be reached, drops the connection or the whole
/// adjustment overruns.
pub async fn tune(
    url: &str,
    switched_on: &[(u64, u16)],
    transition: u16,
    plan: impl Fn(&[Value], &Light) -> Result<Target, Skip>,
) -> Result<Tuned, String> {
    tokio::time::timeout(TUNE_TIMEOUT, async {
        let mut ws = connect(url).await?;
        let result = tune_all(&mut ws, switched_on, transition, plan)
            .await
            .map_err(|e| e.0);
        let _ = ws.close(None).await;
        result
    })
    .await
    .map_err(|_| "timed out".to_owned())?
}

async fn tune_all(
    ws: &mut Socket,
    switched_on: &[(u64, u16)],
    transition: u16,
    plan: impl Fn(&[Value], &Light) -> Result<Target, Skip>,
) -> Result<Tuned, Dropped> {
    let nodes = get_nodes(ws).await?;
    let lights = lights(&nodes);
    let mut decisions: Vec<Result<Target, Decision>> = lights
        .iter()
        .map(|light| plan(&nodes, light).map_err(Decision::Skipped))
        .collect();
    let just_on = |light: &Light| switched_on.contains(&(light.node_id, light.endpoint));
    // Every light to write to is read at once, so one that never answers holds up no other.
    let reads: Vec<_> = lights
        .iter()
        .enumerate()
        .filter(|(i, light)| decisions[*i].is_ok() && !just_on(light))
        .map(|(i, light)| (i, read_on_off(light)))
        .collect();
    for (i, reply) in exchange(ws, reads, Instant::now() + REPLY_TIMEOUT).await? {
        let path = format!("{}/6/0", lights[i].endpoint);
        match reply.filter(|reply| reply.get("error_code").is_none()) {
            None => decisions[i] = Err(Decision::NoResponse),
            Some(reply) if reply["result"][&path] != true => decisions[i] = Err(Decision::Off),
            Some(_) => {}
        }
    }
    let mut commands = Vec::new();
    for (i, light) in lights.iter().enumerate() {
        if let Ok(target) = decisions[i] {
            for command in tune_commands(light, target, just_on(light), transition) {
                commands.push((i, command));
            }
        }
    }
    let sent = commands.len();
    for (i, reply) in exchange(ws, commands, Instant::now() + REPLY_TIMEOUT).await? {
        let decision = match reply {
            None => Decision::NoResponse,
            Some(reply) if reply.get("error_code").is_some() => Decision::Failed,
            Some(_) => continue,
        };
        // No answer outweighs a refusal of the light's other command.
        if decisions[i].is_ok() || decision == Decision::NoResponse {
            decisions[i] = Err(decision);
        }
    }
    Ok(Tuned {
        lights: lights
            .into_iter()
            .zip(decisions)
            .map(|(light, decision)| (light, decision.map_or_else(|d| d, Decision::Sent)))
            .collect(),
        commands: sent,
        nodes,
    })
}

/// The level and colour temperature commands that bring `light` to `target`.
fn tune_commands(light: &Light, target: Target, switched_on: bool, transition: u16) -> Vec<Value> {
    // Just switched on: at once and even if still off (`ExecuteIfOff`).
    let (level_command, transition, options) = if switched_on {
        ("MoveToLevelWithOnOff", 0, 1)
    } else {
        ("MoveToLevel", transition, 0)
    };
    let level = target.level.map(|level| {
        (
            8,
            level_command,
            json!({ "level": level, "transitionTime": transition, "optionsMask": options, "optionsOverride": options }),
        )
    });
    let mireds = target.mireds.map(|mireds| {
        (
            768,
            "MoveToColorTemperature",
            json!({ "colorTemperatureMireds": mireds, "transitionTime": transition, "optionsMask": options, "optionsOverride": options }),
        )
    });
    level
        .into_iter()
        .chain(mireds)
        .map(|(cluster, command, payload)| {
            json!({
                "command": "device_command",
                "args": {
                    "node_id": light.node_id,
                    "endpoint_id": light.endpoint,
                    "cluster_id": cluster,
                    "command_name": command,
                    "payload": payload,
                },
            })
        })
        .collect()
}

/// A read of the light's own On/Off.
fn read_on_off(light: &Light) -> Value {
    json!({
        "command": "read_attribute",
        "args": { "node_id": light.node_id, "attribute_path": [format!("{}/6/0", light.endpoint)] },
    })
}

/// Reads from each light `pick` chooses its own On/Off, with the nodes read.
///
/// # Errors
/// Fails when the server cannot be reached, drops the connection or the reads overrun.
pub async fn read_on(
    url: &str,
    pick: impl Fn(&[Value], &Light) -> bool,
) -> Result<(Vec<Value>, Vec<(Light, Option<bool>)>), String> {
    tokio::time::timeout(TUNE_TIMEOUT, async {
        let mut ws = connect(url).await?;
        let result = async {
            let nodes = get_nodes(&mut ws).await?;
            let picked: Vec<_> = lights(&nodes)
                .into_iter()
                .filter(|l| pick(&nodes, l))
                .collect();
            let reads = picked.iter().map(read_on_off).enumerate().collect();
            let replies = exchange(&mut ws, reads, Instant::now() + REPLY_TIMEOUT).await?;
            let read = picked
                .into_iter()
                .zip(replies)
                .map(|(light, (_, reply))| {
                    let path = format!("{}/6/0", light.endpoint);
                    let on = reply
                        .filter(|reply| reply.get("error_code").is_none())
                        .map(|reply| reply["result"][&path] == true);
                    (light, on)
                })
                .collect();
            Ok((nodes, read))
        }
        .await
        .map_err(|e: Dropped| e.0);
        let _ = ws.close(None).await;
        result
    })
    .await
    .map_err(|_| "timed out".to_owned())?
}

/// Sends every request at once, each under a fresh message id, and returns each one's
/// reply in order, `None` for those unanswered by `deadline`.
async fn exchange<K>(
    ws: &mut Socket,
    requests: Vec<(K, Value)>,
    deadline: Instant,
) -> Result<Vec<(K, Option<Value>)>, Dropped> {
    let mut ids = Vec::with_capacity(requests.len());
    let mut replies = Vec::with_capacity(requests.len());
    for (key, mut request) in requests {
        let id = next_id();
        request["message_id"] = json!(id);
        send(ws, &request).await?;
        ids.push(id);
        replies.push((key, None));
    }
    let mut unanswered = replies.len();
    while unanswered > 0 {
        let Ok(reply) = tokio::time::timeout_at(deadline, receive(ws)).await else {
            break;
        };
        let reply = reply?;
        if let Some(i) = ids.iter().position(|id| reply["message_id"] == id.as_str())
            && replies[i].1.is_none()
        {
            replies[i].1 = Some(reply);
            unanswered -= 1;
        }
    }
    Ok(replies)
}

fn next_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "request-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Sends `request` under a fresh message id and waits for its reply; `None`
/// when none arrives in time. Late replies to earlier requests are skipped.
async fn request(ws: &mut Socket, request: Value) -> Result<Option<Value>, Dropped> {
    request_until(ws, request, Instant::now() + REPLY_TIMEOUT).await
}

/// `request`, waiting for the reply until `deadline`.
pub(crate) async fn request_until(
    ws: &mut Socket,
    mut request: Value,
    deadline: Instant,
) -> Result<Option<Value>, Dropped> {
    let id = next_id();
    request["message_id"] = json!(id);
    send(ws, &request).await?;
    loop {
        let Ok(reply) = tokio::time::timeout_at(deadline, receive(ws)).await else {
            return Ok(None);
        };
        let reply = reply?;
        if reply["message_id"] == id.as_str() {
            return Ok(Some(reply));
        }
    }
}

pub(crate) async fn connect(url: &str) -> Result<Socket, String> {
    let (ws, _) = connect_async(url).await.map_err(|e| e.to_string())?;
    Ok(ws)
}

pub(crate) async fn send(ws: &mut Socket, request: &Value) -> Result<(), Dropped> {
    ws.send(Message::text(request.to_string()))
        .await
        .map_err(|e| Dropped(e.to_string()))
}

/// The next reply or event; JSON that does not parse is a protocol failure.
pub(crate) async fn receive(ws: &mut Socket) -> Result<Value, Dropped> {
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
        if reply["message_id"] == "get_nodes" {
            return nodes_of(&mut reply);
        }
    }
}

fn nodes_of(reply: &mut Value) -> Result<Vec<Value>, Dropped> {
    match reply["result"].take() {
        Value::Array(nodes) => Ok(nodes),
        _ => Err(Dropped(format!(
            "get_nodes failed: {}",
            reply["error_code"]
        ))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Light, lights, locate, network};

    // Trimmed from a matterjs-server `get_nodes` result (2026-10-05).
    fn nodes() -> Vec<serde_json::Value> {
        vec![
            json!({ "node_id": 1, "attributes": {
                "0/40/1": "Aqara", "0/40/15": "54ef4475b119",
                "2/57/1": "Aqara", "2/57/15": "54ef44100126e0a6",
                "3/57/1": "Aqara", "3/57/15": "54ef441001372c4c",
            }}),
            json!({ "node_id": 5, "attributes": { "0/40/1": "Tapo", "0/40/15": "AABBCCDDEEFF" }}),
            json!({ "node_id": 16, "attributes": {
                "0/40/1": "Uascent", "0/40/15": "U20251209000001453",
            }}),
        ]
    }

    #[test]
    fn locates_root_devices_by_basic_information() {
        assert_eq!(locate(&nodes(), "Tapo", "AABBCCDDEEFF"), Some((5, 0)));
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
        assert_eq!(locate(&nodes(), "Tapo", "AABBCCDD0000"), None);
        assert_eq!(locate(&nodes(), "Aqara", "AABBCCDDEEFF"), None);
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
            levels: None,
            mireds: None,
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
        // Level and colour temperature ranges as the light reports them, else the usual ones.
        let tunable = vec![json!({ "node_id": 7, "attributes": {
            "1/29/0": [{ "0": 269, "1": 1 }], "1/6/0": true,
            "1/8/0": 9, "1/8/2": 0, "1/768/65532": 25, "1/768/16395": 142, "1/768/16396": 454,
            "2/29/0": [{ "0": 268, "1": 1 }], "2/6/0": true, "2/8/0": 9, "2/768/65532": 16,
            // Hue and saturation only: no colour temperature.
            "3/29/0": [{ "0": 269, "1": 1 }], "3/6/0": true, "3/768/65532": 1,
        }})];
        let ranges: Vec<_> = lights(&tunable)
            .into_iter()
            .map(|l| (l.levels, l.mireds))
            .collect();
        assert_eq!(
            ranges,
            [
                (Some((0, 254)), Some((142, 454))),
                (Some((1, 254)), Some((153, 370))),
                (None, None),
            ]
        );
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

    #[test]
    fn a_nodes_network_is_the_one_its_network_commissioning_runs_on() {
        let nodes = vec![
            json!({ "node_id": 1, "attributes": { "0/49/65532": 1 }}),
            json!({ "node_id": 2, "attributes": { "0/49/65532": 2 }}),
            json!({ "node_id": 3, "attributes": { "0/49/65532": 4 }}),
            json!({ "node_id": 4, "attributes": { "0/40/1": "Plainco" }}),
        ];
        assert_eq!(network(&nodes, 1), Some("wifi"));
        assert_eq!(network(&nodes, 2), Some("thread"));
        assert_eq!(network(&nodes, 3), Some("ethernet"));
        assert_eq!(network(&nodes, 4), None);
        assert_eq!(network(&nodes, 9), None);
    }
}
