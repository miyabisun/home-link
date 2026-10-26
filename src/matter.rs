use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

pub use crate::link::{Fault, Outcome};
use crate::{
    link::{self, Link, next_id},
    schedule::{Skip, Target},
};

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

/// Reads every node from the matterjs-server WebSocket API at `url` by `deadline`.
///
/// # Errors
/// Fails when the server cannot be reached, times out, or answers with an error.
pub async fn fetch_nodes(url: &str, deadline: Instant) -> Result<Vec<Value>, Fault> {
    let mut link = Link::open(url, deadline).await?;
    let nodes = link.nodes(deadline).await;
    link.close().await;
    nodes
}

/// A switch: the nodes read in its session and each device's outcome.
pub struct Switched {
    pub nodes: Vec<Value>,
    pub results: Vec<(Light, Outcome)>,
    /// Commands went out, and matterjs-server answered none of them: neither an
    /// acceptance nor an error came back for any device, though one may since show switched.
    pub none_answered: bool,
}

/// Sends On or Off to every reachable light but those `ignored` and reports each light's
/// outcome, with the nodes read in the same session.
///
/// All lights are sent their command at once, so a light that never answers holds up none of
/// the others. Those still unanswered after `RESEND_AFTER` are sent it again with the nodes
/// read again, and whatever has answered by `ANSWER_WINDOW` is reported: an answer to either
/// send counts. A light still unanswered then, whose On/Off the nodes read once more show
/// changed to the one sent, switched too. A connection dropped mid-session is opened again
/// and the command sent again to the lights yet to answer (On and Off are absolute) within
/// the same window; a server that refuses the first connection fails at once. Nothing runs
/// past `deadline`.
///
/// # Errors
/// Fails when the server cannot be reached or its nodes cannot be read in time.
pub async fn switch(
    url: &str,
    deadline: Instant,
    on: bool,
    ignored: impl Fn(&[Value], &Light) -> bool,
) -> Result<Switched, Fault> {
    switch_among(url, deadline, on, lights, ignored).await
}

/// `switch` over the endpoints `targets` picks from the nodes, lights or not.
///
/// # Errors
/// As `switch`.
pub async fn switch_among(
    url: &str,
    deadline: Instant,
    on: bool,
    targets: impl Fn(&[Value]) -> Vec<Light>,
    ignored: impl Fn(&[Value], &Light) -> bool,
) -> Result<Switched, Fault> {
    let mut link = Link::open(url, deadline).await?;
    let nodes = match link.nodes(deadline).await {
        Ok(nodes) => nodes,
        Err(fault) => {
            link.close().await;
            return Err(fault);
        }
    };
    let mut switching = Switching::new(nodes, on, &targets, &ignored);
    let read_at = Instant::now();
    let window = deadline.min(read_at + ANSWER_WINDOW);
    let resend_at = read_at + RESEND_AFTER;
    loop {
        let result = switching.run(&mut link, resend_at, window, &targets).await;
        let result = match result {
            Ok(()) if switching.settled() => Ok(()),
            // Whatever answers while the nodes are read once more still counts.
            Ok(()) => {
                let late = deadline.min(Instant::now() + LATE_READ);
                switching.confirm(&mut link, late).await
            }
            Err(fault) => Err(fault),
        };
        link.close().await;
        let Err(Fault::Dropped(error)) = result else {
            break;
        };
        tracing::warn!(%error, "matterjs-server connection dropped; reconnecting");
        let Some(again) = reconnect(url, window).await else {
            break;
        };
        link = again;
    }
    Ok(switching.finish())
}

/// A new connection to `url` before `deadline`, tried every `RECONNECT_DELAY`.
async fn reconnect(url: &str, deadline: Instant) -> Option<Link> {
    loop {
        tokio::time::sleep_until(deadline.min(Instant::now() + RECONNECT_DELAY)).await;
        if Instant::now() >= deadline {
            return None;
        }
        if let Ok(link) = Link::open(url, deadline).await {
            return Some(link);
        }
    }
}

const RECONNECT_DELAY: Duration = Duration::from_millis(500);

/// One switch across connections: each light's outcome so far and the sends awaiting an answer.
struct Switching {
    nodes: Vec<Value>,
    on: bool,
    /// Each light with its outcome once there is one.
    results: Vec<(Light, Option<Outcome>)>,
    /// Each light's On/Off as first read.
    before: Vec<Option<bool>>,
    /// Message id of each send awaiting an answer on the current connection, and its light.
    awaiting: std::collections::HashMap<String, usize>,
    sends: Vec<usize>,
    resent: bool,
    /// Some command was answered, accepted or not.
    replied: bool,
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
                let outcome = ignored(&nodes, &light).then_some(Outcome::Ignored);
                (light, outcome)
            })
            .collect();
        Self {
            sends: vec![0; results.len()],
            before: results.iter().map(|(light, _)| light.on).collect(),
            nodes,
            results,
            on,
            awaiting: std::collections::HashMap::new(),
            resent: false,
            replied: false,
        }
    }

    fn settled(&self) -> bool {
        self.results.iter().all(|(_, o)| o.is_some())
    }

    /// Sends to every reachable light yet to answer that has been sent fewer than `ATTEMPTS`
    /// commands, then takes answers until all have answered or `window` ends, sending again
    /// once at `resend_at` with the nodes read again.
    async fn run(
        &mut self,
        link: &mut Link,
        resend_at: Instant,
        window: Instant,
        targets: &impl Fn(&[Value]) -> Vec<Light>,
    ) -> Result<(), Fault> {
        // Sends on an earlier connection will not be answered on this one.
        self.awaiting.clear();
        self.send(link, window).await?;
        while !self.settled() {
            let wait = if self.resent {
                window
            } else {
                resend_at.min(window)
            };
            match link.next(wait).await? {
                Some(reply) => self.answer(&reply),
                None if wait == window => break,
                None => {
                    self.resent = true;
                    if !self.reread(link, window, targets).await? {
                        break;
                    }
                    self.send(link, window).await?;
                }
            }
        }
        Ok(())
    }

    async fn send(&mut self, link: &mut Link, deadline: Instant) -> Result<(), Fault> {
        let command = if self.on { "On" } else { "Off" };
        for (index, (light, outcome)) in self.results.iter().enumerate() {
            if !light.reachable || outcome.is_some() || self.sends[index] >= ATTEMPTS {
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
                    "command_name": command,
                    "payload": {},
                },
            });
            link.send(&request, deadline).await?;
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
        self.replied = true;
        let outcome = &mut self.results[index].1;
        if outcome.is_none() {
            *outcome = Some(Outcome::of(Some(reply)));
        }
    }

    /// Reads the nodes again, taking answers that arrive meanwhile; `None` when the nodes do
    /// not come by `deadline`.
    async fn read_nodes(
        &mut self,
        link: &mut Link,
        deadline: Instant,
    ) -> Result<Option<Vec<Value>>, Fault> {
        let id = next_id();
        link.send(
            &json!({ "message_id": id, "command": "get_nodes" }),
            deadline,
        )
        .await?;
        while let Some(reply) = link.next(deadline).await? {
            if reply["message_id"] != id.as_str() {
                self.answer(&reply);
                continue;
            }
            return Ok(link::nodes_of(reply).ok());
        }
        Ok(None)
    }

    /// Reads the nodes again and refreshes the lights yet to answer; `false` when the nodes
    /// do not come by `deadline`.
    async fn reread(
        &mut self,
        link: &mut Link,
        deadline: Instant,
        targets: &impl Fn(&[Value]) -> Vec<Light>,
    ) -> Result<bool, Fault> {
        let Some(nodes) = self.read_nodes(link, deadline).await? else {
            return Ok(false);
        };
        for light in targets(&nodes) {
            let key = (light.node_id, light.endpoint);
            if let Some((known, _)) = self
                .results
                .iter_mut()
                .find(|(l, o)| (l.node_id, l.endpoint) == key && o.is_none())
            {
                *known = light;
            }
        }
        self.nodes = nodes;
        Ok(true)
    }

    /// Reads the nodes once more by `deadline`, and counts as switched each light sent the
    /// command and yet to answer whose On/Off they show changed to the one sent: its answer
    /// came too late, or never, but matterjs-server has heard from it since.
    async fn confirm(&mut self, link: &mut Link, deadline: Instant) -> Result<(), Fault> {
        if !(0..self.results.len()).any(|i| self.results[i].1.is_none() && self.sends[i] > 0) {
            return Ok(());
        }
        let Some(nodes) = self.read_nodes(link, deadline).await? else {
            tracing::warn!(
                "matterjs-server did not serve the nodes again in time; late answers uncounted"
            );
            return Ok(());
        };
        for (i, (light, outcome)) in self.results.iter_mut().enumerate() {
            let reported = nodes
                .iter()
                .find(|n| n["node_id"] == light.node_id)
                .and_then(|n| n["attributes"][format!("{}/6/0", light.endpoint)].as_bool());
            if outcome.is_none()
                && self.sends[i] > 0
                && self.before[i] != Some(self.on)
                && reported == Some(self.on)
            {
                tracing::info!(
                    node_id = light.node_id,
                    endpoint = light.endpoint,
                    "device answered late; its reported On/Off shows it switched"
                );
                *outcome = Some(Outcome::Accepted);
            }
        }
        Ok(())
    }

    /// Each light's outcome, those yet to answer as not answered or, never sent, not sent,
    /// logging every one not accepted and a server that answered none.
    fn finish(self) -> Switched {
        let sent = self.sends.iter().filter(|s| **s > 0).count();
        let results: Vec<_> = self
            .results
            .into_iter()
            .zip(&self.sends)
            .map(|((light, outcome), sends)| {
                let outcome = outcome.unwrap_or(if *sends > 0 {
                    Outcome::NoAnswer
                } else {
                    Outcome::NotSent
                });
                outcome.warn("switch", light.node_id, light.endpoint);
                (light, outcome)
            })
            .collect();
        let none_answered = sent > 0 && !self.replied;
        if none_answered {
            tracing::warn!(sent, "matterjs-server answered none of the commands");
        }
        Switched {
            nodes: self.nodes,
            results,
            none_answered,
        }
    }
}

/// At most two sends to a light that has not answered, the second after `RESEND_AFTER`; the
/// switch reports what has answered by `ANSWER_WINDOW` after the nodes are read, and what
/// the nodes read once more within `LATE_READ` after it show switched.
const ATTEMPTS: usize = 2;
const RESEND_AFTER: Duration = Duration::from_secs(1);
const ANSWER_WINDOW: Duration = Duration::from_millis(2500);
const LATE_READ: Duration = Duration::from_millis(400);

/// Writes `steps` to `endpoint` of `node_id` in order, stopping at the first one not
/// accepted, and reports it, or `Accepted` once all are; nothing runs past `deadline`.
///
/// # Errors
/// Fails when the server cannot be reached or drops the connection.
pub async fn drive(
    url: &str,
    deadline: Instant,
    node_id: u64,
    endpoint: u16,
    steps: &[crate::aircon::Step],
) -> Result<Outcome, Fault> {
    let mut link = Link::open(url, deadline).await?;
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
            let by = deadline.min(Instant::now() + REPLY_TIMEOUT);
            let outcome = Outcome::of(link.request(command, by).await?.as_ref());
            if outcome != Outcome::Accepted {
                return Ok(outcome);
            }
        }
        Ok(Outcome::Accepted)
    }
    .await;
    link.close().await;
    if let Ok(outcome) = &result {
        outcome.warn("drive", node_id, endpoint);
    }
    result
}

/// What became of one light in an adjustment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Left alone before its On/Off was read.
    Skipped(Skip),
    /// Read as off just before writing: nothing sent.
    Off,
    /// Not read or written: not sent, no answer, or an error to the read.
    NoResponse(Outcome),
    /// A command not accepted: matterjs-server's error or the device's refusal.
    Failed(Outcome),
    Sent(Target),
}

/// An adjustment: the nodes read, each light's decision and the commands sent.
pub struct Tuned {
    pub nodes: Vec<Value>,
    pub lights: Vec<(Light, Decision)>,
    pub commands: usize,
}

const REPLY_TIMEOUT: Duration = Duration::from_secs(3);
/// Each scheduled change fades over 30 s (tenths of a second) so a step every ten minutes goes unnoticed.
pub const TRANSITION: u16 = 300;

/// Reads every light, asks `plan` what to send each given the nodes read, reads the On/Off of each
/// light to write to again, and sends the level and colour temperature only to
/// lights read as on, fading over `transition` tenths of a second. Commands
/// never carry `ExecuteIfOff`, so a light switched off in between ignores them
/// and stays off. Nothing runs past `deadline`.
///
/// A light in `switched_on`, just switched on, is not read: its On may not be
/// reported yet. It gets the values at once, lighting it if still off.
///
/// # Errors
/// Fails when the server cannot be reached, drops the connection or its nodes cannot be
/// read in time.
pub async fn tune(
    url: &str,
    deadline: Instant,
    switched_on: &[(u64, u16)],
    transition: u16,
    plan: impl Fn(&[Value], &Light) -> Result<Target, Skip>,
) -> Result<Tuned, Fault> {
    let mut link = Link::open(url, deadline).await?;
    let result = tune_all(&mut link, deadline, switched_on, transition, plan).await;
    link.close().await;
    let tuned = result?;
    for (light, decision) in &tuned.lights {
        if let Decision::NoResponse(outcome) | Decision::Failed(outcome) = decision {
            outcome.warn("tune", light.node_id, light.endpoint);
        }
    }
    Ok(tuned)
}

async fn tune_all(
    link: &mut Link,
    deadline: Instant,
    switched_on: &[(u64, u16)],
    transition: u16,
    plan: impl Fn(&[Value], &Light) -> Result<Target, Skip>,
) -> Result<Tuned, Fault> {
    let nodes = link.nodes(deadline).await?;
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
    let by = deadline.min(Instant::now() + REPLY_TIMEOUT);
    for (i, reply) in link.exchange(reads, by).await? {
        let path = format!("{}/6/0", lights[i].endpoint);
        match Outcome::of(reply.as_ref()) {
            Outcome::Accepted if reply.is_some_and(|r| r["result"][&path] == true) => {}
            Outcome::Accepted => decisions[i] = Err(Decision::Off),
            missed => decisions[i] = Err(Decision::NoResponse(missed)),
        }
    }
    let mut commands = Vec::new();
    for (i, light) in lights.iter().enumerate() {
        if let Ok(target) = &decisions[i] {
            for command in tune_commands(light, *target, just_on(light), transition) {
                commands.push((i, command));
            }
        }
    }
    let sent = commands.len();
    let by = deadline.min(Instant::now() + REPLY_TIMEOUT);
    for (i, reply) in link.exchange(commands, by).await? {
        let decision = match Outcome::of(reply.as_ref()) {
            Outcome::Accepted => continue,
            Outcome::NoAnswer => Decision::NoResponse(Outcome::NoAnswer),
            refused => Decision::Failed(refused),
        };
        // No answer outweighs a refusal of the light's other command.
        if decisions[i].is_ok() || matches!(decision, Decision::NoResponse(_)) {
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
pub(crate) fn read_on_off(light: &Light) -> Value {
    json!({
        "command": "read_attribute",
        "args": { "node_id": light.node_id, "attribute_path": [format!("{}/6/0", light.endpoint)] },
    })
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
