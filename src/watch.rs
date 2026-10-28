//! Keeping watch on each light's connection: when a light counts as down, when to ask
//! matterjs-server to reconnect it, and what asks for the user. Nothing here talks to
//! matterjs-server or the database; [`Watch::step`] decides from what one check saw.
//!
//! A check runs every minute on the minute. It reads every node's `available` and, every
//! [`READ_EVERY`], reads each light's On/Off for real, since a light can stay `available`
//! after it stopped answering. A light is down while it is not `available` or after
//! [`MISSES_TO_RECONNECT`] reads in a row went unanswered.
//!
//! A ledger light matterjs-server no longer serves has left the fabric: no reconnection brings
//! it back, so it is watched by its ledger row until it is registered again.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::{
    meter::care::Trouble,
    rejoin::{Leaves, Pursuit, Reason},
};

/// Every light's On/Off is read for real this often, in seconds.
pub const READ_EVERY: i64 = 300;
/// matterjs-server out of reach this long, in seconds, asks for the user; so does a light out
/// of the fabric this long for a reason the user can act on.
pub const ASK_AFTER: i64 = 300;
/// At most this many lights are asked to reconnect in one check, so lights that all go down
/// at once, as when matterjs-server or the Thread border router restarts, do not flood it.
pub const MAX_RECONNECTS_PER_CHECK: usize = 3;
/// One light is asked to reconnect at most this often, in seconds.
pub const RECONNECT_EVERY: i64 = 600;
/// Reads in a row a light that stays `available` leaves unanswered before it is down.
pub const MISSES_TO_RECONNECT: u32 = 2;
/// Reconnections after which a light still down asks to be switched off and on at the wall.
pub const RECONNECTS_TO_ADVISE: u32 = 3;

/// A light: its node and endpoint.
pub type Key = (u64, u16);

/// One light as a check saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub key: Key,
    pub name: String,
    /// `wifi`, `thread` or `ethernet`, when known.
    pub network: Option<&'static str>,
    /// The ledger row it is, if any.
    pub device: Option<i64>,
    /// The ledger ignores it: it is not watched.
    pub ignored: bool,
    /// matterjs-server serves it as available and, behind a bridge, the bridge reaches it.
    pub available: bool,
    /// Whether it answered a read of its On/Off, when one was sent in this check.
    pub answered: Option<bool>,
}

/// A ledger light matterjs-server does not serve: it left the fabric, and only registering it
/// again brings it back. What is known of it is from when it was last served.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Left {
    /// Its ledger row.
    pub device: i64,
    pub name: String,
    pub labels: Vec<String>,
    pub serial: Option<String>,
    /// The ledger ignores it: it is not watched.
    pub ignored: bool,
    /// The network it was last on, when known.
    pub network: Option<&'static str>,
    /// Its last Software Version (`0/40/10`, or `57/10` behind a bridge).
    pub software_version: Option<String>,
    /// Its last Thread parent, as an RLOC16, when it was a child.
    pub thread_parent: Option<String>,
    /// When matterjs-server sent that it removed the node, if it did.
    pub removed_at: Option<i64>,
    /// When it left the fabric before, as far as the history goes.
    pub leaves: Vec<i64>,
}

/// How the time a light left the fabric is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeftBy {
    /// matterjs-server sent that it removed the node: the time it did.
    Event,
    /// A check found it gone that the one before saw: the time of that check.
    Check,
    /// It was gone before the watch first saw it: the time the watch found it.
    Unknown,
}

impl LeftBy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Check => "check",
            Self::Unknown => "unknown",
        }
    }
}

/// How a light was found down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// matterjs-server serves it as not available.
    Unavailable,
    /// It stayed available, but did not answer reads of its On/Off.
    NoAnswer,
    /// matterjs-server itself was out of reach.
    Unreachable,
    /// matterjs-server no longer serves it: it left the fabric.
    #[serde(rename = "left_fabric")]
    Left,
}

impl Cause {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::NoAnswer => "no_answer",
            Self::Unreachable => "unreachable",
            Self::Left => "left_fabric",
        }
    }
}

/// One stretch a light, or matterjs-server, was out of reach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Outage {
    /// The light's node and endpoint, or `None` for matterjs-server.
    pub node_id: Option<u64>,
    pub endpoint: Option<u16>,
    pub name: String,
    pub network: Option<&'static str>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub found_by: Cause,
    /// Times matterjs-server was asked to reconnect the light.
    pub reconnects: u32,
    /// What the last of those asks came to.
    pub reconnect_result: Option<String>,
    /// Whether the light's addresses answered a ping, when checked.
    pub reachable: Option<bool>,
    /// The ledger row of the light, when known.
    pub device_id: Option<i64>,
    /// When it left the fabric, and how that time is known.
    pub left_at: Option<i64>,
    pub left_by: Option<LeftBy>,
    /// Its labels, serial number, last Software Version and Thread parent, kept when it left.
    pub labels: Vec<String>,
    pub serial_number: Option<String>,
    pub software_version: Option<String>,
    pub thread_parent: Option<String>,
    /// For a light that left: home-link registering it again, and how often it left.
    pub rejoin: Pursuit,
    pub leaves: Leaves,
}

impl Outage {
    #[must_use]
    pub fn new(
        key: Option<Key>,
        name: &str,
        network: Option<&'static str>,
        at: i64,
        cause: Cause,
    ) -> Self {
        Self {
            node_id: key.map(|(node, _)| node),
            endpoint: key.map(|(_, endpoint)| endpoint),
            name: name.to_owned(),
            network,
            started_at: at,
            ended_at: None,
            found_by: cause,
            reconnects: 0,
            reconnect_result: None,
            reachable: None,
            device_id: None,
            left_at: None,
            left_by: None,
            labels: Vec::new(),
            serial_number: None,
            software_version: None,
            thread_parent: None,
            rejoin: Pursuit::default(),
            leaves: Leaves::default(),
        }
    }

    /// The light, or `None` for matterjs-server.
    #[must_use]
    pub fn key(&self) -> Option<Key> {
        self.node_id.zip(self.endpoint)
    }
}

/// What the watch knows of one light.
#[derive(Clone, Debug, Default)]
struct Light {
    name: String,
    /// Its ledger row, if any: a light that leaves the fabric is watched by it.
    device: Option<i64>,
    network: Option<&'static str>,
    /// Since when it has been connected, as far as the watch saw.
    up_since: Option<i64>,
    last_answer: Option<i64>,
    last_read: Option<i64>,
    /// Unanswered reads in a row, and when the first of them was sent.
    misses: u32,
    first_miss: Option<i64>,
    last_reconnect: Option<i64>,
    outage: Option<Outage>,
}

/// A change worth a log line and a write to the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A stretch began.
    Down(Outage),
    /// A light left the fabric: its stretch began then, or it was down and is now out of it.
    Left(Outage),
    /// A stretch still open was reconnected, pinged, or its pursuit moved on.
    Updated(Outage),
    /// A stretch ended: back, or no longer watched.
    Up(Outage),
}

/// A state that asks for the user, as the admin screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attention {
    /// matterjs-server out of reach for [`ASK_AFTER`].
    ServerDown { since: i64 },
    /// A light still down after [`RECONNECTS_TO_ADVISE`] reconnections: switch it off and on.
    PowerCycle {
        name: String,
        device_id: Option<i64>,
        since: i64,
    },
    /// A light out of the fabric for a reason the user can act on, that stood
    /// [`crate::rejoin::ASK_AFTER`].
    #[serde(rename = "left_fabric")]
    Left {
        name: String,
        device_id: i64,
        left_at: i64,
        left_by: Option<LeftBy>,
        reason: Reason,
    },
    /// The meter unheard for [`crate::meter::care::ASK_AFTER`].
    MeterLost {
        since: i64,
        trouble: Trouble,
        battery: Option<u8>,
    },
    /// The meter's battery below [`crate::meter::care::LOW_BATTERY`].
    MeterBattery { battery: u8 },
}

/// What one check calls for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// Lights to ask matterjs-server to reconnect.
    pub reconnect: Vec<Key>,
    /// Lights whose network reach to check, being down on Wi-Fi.
    pub ping: Vec<Key>,
    pub changes: Vec<Change>,
}

/// The watch over every light and matterjs-server.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    lights: BTreeMap<Key, Light>,
    /// The stretches of ledger lights that left the fabric, by ledger row.
    left: BTreeMap<i64, Outage>,
    server: Option<Outage>,
    server_up_since: Option<i64>,
    checked_at: Option<i64>,
}

/// A watched light's node, else its ledger row, name, and state: `connected`, `down` or `left_fabric`.
pub type Connected = (Option<Key>, Option<i64>, String, &'static str);

/// One light's state for the API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LightStatus {
    /// `None` for a light that left the fabric: matterjs-server serves no node for it.
    pub node_id: Option<u64>,
    pub endpoint: Option<u16>,
    /// The ledger row, when it has one.
    pub device_id: Option<i64>,
    pub name: String,
    /// `None` for a light that left the fabric; the network it was on is in its `outage`.
    pub network: Option<&'static str>,
    pub connected: bool,
    /// `connected`, `down`, or `left_fabric`.
    pub state: &'static str,
    /// Since when it has been connected, or down.
    pub since: Option<i64>,
    pub last_answer: Option<i64>,
    pub outage: Option<Outage>,
}

/// The watch's state for the API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub checked_at: Option<i64>,
    /// Whether the last check reached matterjs-server.
    pub server_reachable: Option<bool>,
    pub server_since: Option<i64>,
    pub server_outage: Option<Outage>,
    /// Every watched Wi-Fi light is down, and there is one.
    pub wifi_all_down: bool,
    pub lights: Vec<LightStatus>,
}

impl Watch {
    /// Whether this check should read `key`'s On/Off for real: one never read, one whose last
    /// read went unanswered, and every [`READ_EVERY`].
    #[must_use]
    pub fn read_due(&self, at: i64, key: Key) -> bool {
        self.lights.get(&key).is_none_or(|light| {
            light.misses > 0 || light.last_read.is_none_or(|read| at - read >= READ_EVERY)
        })
    }

    /// Takes in one check at `at`, on the minute: the lights it saw, or `None` when
    /// matterjs-server was out of reach.
    pub fn step(&mut self, at: i64, seen: Option<&[Seen]>, left: &[Left]) -> Step {
        self.checked_at = Some(at);
        let mut step = Step::default();
        match seen {
            None => self.server_down(at, &mut step),
            Some(seen) => {
                self.server_up(at, &mut step);
                self.lights_seen(at, seen, left, &mut step);
            }
        }
        step
    }

    fn server_down(&mut self, at: i64, step: &mut Step) {
        self.server_up_since = None;
        self.server.get_or_insert_with(|| {
            let outage = Outage::new(None, "matterjs-server", None, at, Cause::Unreachable);
            step.changes.push(Change::Down(outage.clone()));
            outage
        });
    }

    fn server_up(&mut self, at: i64, step: &mut Step) {
        self.server_up_since.get_or_insert(at);
        if let Some(mut outage) = self.server.take() {
            outage.ended_at = Some(at);
            step.changes.push(Change::Up(outage));
        }
    }

    fn lights_seen(&mut self, at: i64, seen: &[Seen], left: &[Left], step: &mut Step) {
        let watched: Vec<&Seen> = seen.iter().filter(|light| !light.ignored).collect();
        let gone: Vec<&Left> = left.iter().filter(|light| !light.ignored).collect();
        self.leave(at, &gone, step);
        self.come_back(at, &gone, step);
        self.forget(at, seen, &watched, step);
        let mut due = Vec::new();
        for seen in watched {
            let light = self.lights.entry(seen.key).or_default();
            light.name.clone_from(&seen.name);
            light.network = seen.network;
            light.device = seen.device;
            match seen.answered {
                Some(true) => {
                    light.last_read = Some(at);
                    light.last_answer = Some(at);
                    light.misses = 0;
                    light.first_miss = None;
                }
                Some(false) => {
                    light.last_read = Some(at);
                    light.misses += 1;
                    light.first_miss.get_or_insert(at);
                }
                None => {}
            }
            if seen.available && light.misses < MISSES_TO_RECONNECT {
                if let Some(mut outage) = light.outage.take() {
                    outage.ended_at = Some(at);
                    step.changes.push(Change::Up(outage));
                }
                light.up_since.get_or_insert(at);
                continue;
            }
            light.up_since = None;
            light.outage.get_or_insert_with(|| {
                let (cause, since) = if seen.available {
                    (Cause::NoAnswer, light.first_miss.unwrap_or(at))
                } else {
                    (Cause::Unavailable, at)
                };
                let outage = Outage::new(Some(seen.key), &seen.name, seen.network, since, cause);
                step.changes.push(Change::Down(outage.clone()));
                outage
            });
            if light
                .last_reconnect
                .is_none_or(|last| at - last >= RECONNECT_EVERY)
            {
                due.push((light.last_reconnect, seen.key));
            }
        }
        self.reconnect(at, due, step);
    }

    /// Ledger lights matterjs-server no longer serves have left the fabric: a light watched
    /// keeps its stretch, or starts one, from when matterjs-server removed it, else from this
    /// check; one never seen starts one now. It is watched by its ledger row from then on.
    fn leave(&mut self, at: i64, gone: &[&Left], step: &mut Step) {
        for left in gone {
            if let Some(outage) = self.left.get_mut(&left.device) {
                outage.name.clone_from(&left.name);
                outage.labels.clone_from(&left.labels);
                if outage.rejoin.still_gone(at) {
                    step.changes.push(Change::Updated(outage.clone()));
                }
                continue;
            }
            let watched = self
                .lights
                .iter()
                .find(|(_, light)| light.device == Some(left.device))
                .map(|(key, _)| *key);
            let (left_at, left_by) = match (left.removed_at, watched) {
                (Some(removed), _) => (removed, LeftBy::Event),
                (None, Some(_)) => (at, LeftBy::Check),
                (None, None) => (at, LeftBy::Unknown),
            };
            let light = watched.and_then(|key| self.lights.remove(&key));
            let network = light
                .as_ref()
                .and_then(|light| light.network)
                .or(left.network);
            let mut outage = light
                .and_then(|light| light.outage)
                .unwrap_or_else(|| Outage::new(watched, &left.name, network, left_at, Cause::Left));
            outage.device_id = Some(left.device);
            outage.left_at = Some(left_at);
            outage.left_by = Some(left_by);
            outage.network = network;
            outage.labels.clone_from(&left.labels);
            outage.serial_number.clone_from(&left.serial);
            outage.software_version.clone_from(&left.software_version);
            outage.thread_parent.clone_from(&left.thread_parent);
            outage.leaves = Leaves::of(&left.leaves, left_at);
            step.changes.push(Change::Left(outage.clone()));
            self.left.insert(left.device, outage);
        }
    }

    /// A light that left and is served again, registered again, ends its stretch; so does one
    /// no longer in the ledger as a light to watch.
    fn come_back(&mut self, at: i64, gone: &[&Left], step: &mut Step) {
        let ended: Vec<i64> = self
            .left
            .keys()
            .filter(|device| !gone.iter().any(|left| left.device == **device))
            .copied()
            .collect();
        for device in ended {
            let Some(mut outage) = self.left.remove(&device) else {
                continue;
            };
            outage.ended_at = Some(at);
            step.changes.push(Change::Up(outage));
        }
    }

    /// A light ignored since ends its stretch. One matterjs-server no longer serves
    /// and the ledger does not hold stays as it was while down, and is no longer watched while up.
    fn forget(&mut self, at: i64, seen: &[Seen], watched: &[&Seen], step: &mut Step) {
        let gone: Vec<Key> = self
            .lights
            .iter()
            .filter(|(key, light)| {
                !watched.iter().any(|light| light.key == **key)
                    && (light.outage.is_none() || seen.iter().any(|light| light.key == **key))
            })
            .map(|(key, _)| *key)
            .collect();
        for key in gone {
            if let Some(mut outage) = self.lights.remove(&key).and_then(|light| light.outage) {
                outage.ended_at = Some(at);
                step.changes.push(Change::Up(outage));
            }
        }
    }

    /// Asks the lights `due` to reconnect, those waiting longest first and at most
    /// [`MAX_RECONNECTS_PER_CHECK`]; the rest wait for the next check.
    fn reconnect(&mut self, at: i64, mut due: Vec<(Option<i64>, Key)>, step: &mut Step) {
        due.sort();
        for (_, key) in due.into_iter().take(MAX_RECONNECTS_PER_CHECK) {
            let light = self.lights.get_mut(&key).expect("a light due is watched");
            light.last_reconnect = Some(at);
            if let Some(outage) = light.outage.as_mut() {
                outage.reconnects += 1;
                step.changes.push(Change::Updated(outage.clone()));
            }
            step.reconnect.push(key);
            if light.network == Some("wifi") {
                step.ping.push(key);
            }
        }
    }

    fn outage_mut(&mut self, key: Key) -> Option<&mut Outage> {
        self.lights.get_mut(&key)?.outage.as_mut()
    }

    /// When the ledger light `device` left the fabric, while it is out of it.
    #[must_use]
    pub fn left_at(&self, device: i64) -> Option<i64> {
        self.left.get(&device).and_then(|outage| outage.left_at)
    }

    /// The ledger rows of the lights out of the fabric, to look for.
    #[must_use]
    pub fn pursued(&self) -> Vec<i64> {
        self.left
            .iter()
            .filter(|(_, outage)| !outage.rejoin.rejoined)
            .map(|(device, _)| *device)
            .collect()
    }

    /// Takes in a look at `at` for the light that left as ledger row `device`: whether the ledger
    /// holds its code, whether Bluetooth worked, and whether its advertisement was heard.
    pub fn look(&mut self, device: i64, at: i64, code: bool, bluetooth: bool, heard: bool) {
        if let Some(outage) = self.left.get_mut(&device) {
            let since = outage.left_at.unwrap_or(outage.started_at);
            outage.rejoin.look(at, since, code, bluetooth, heard);
        }
    }

    /// The lights that left to commission now, with when each was last tried.
    #[must_use]
    pub fn due(&self, at: i64) -> Vec<(i64, Option<i64>)> {
        self.left
            .iter()
            .filter(|(_, outage)| outage.rejoin.due(at))
            .map(|(device, outage)| (*device, outage.rejoin.attempted_at))
            .collect()
    }

    /// Records a commissioning of the light that left as ledger row `device`, ended at `at`.
    pub fn attempted(
        &mut self,
        device: i64,
        at: i64,
        result: Result<(), Reason>,
    ) -> Option<Change> {
        let outage = self.left.get_mut(&device)?;
        outage.rejoin.attempted(at, result);
        Some(Change::Updated(outage.clone()))
    }

    /// Takes back the stretches still open when home-link last stopped.
    pub fn resume(&mut self, outages: Vec<Outage>) {
        for outage in outages {
            if let Some(device) = outage.device_id.filter(|_| outage.left_at.is_some()) {
                self.left.insert(device, outage);
                continue;
            }
            match outage.key() {
                None => self.server = Some(outage),
                Some(key) => {
                    self.lights.insert(
                        key,
                        Light {
                            name: outage.name.clone(),
                            device: outage.device_id,
                            network: outage.network,
                            outage: Some(outage),
                            ..Light::default()
                        },
                    );
                }
            }
        }
    }

    /// Records what asking matterjs-server to reconnect `key` came to.
    pub fn reconnected(&mut self, key: Key, result: String) -> Option<Change> {
        let outage = self.outage_mut(key)?;
        outage.reconnect_result = Some(result);
        Some(Change::Updated(outage.clone()))
    }

    /// Records whether `key`'s addresses answered a ping.
    pub fn reached(&mut self, key: Key, reachable: bool) -> Option<Change> {
        let outage = self.outage_mut(key)?;
        outage.reachable = Some(reachable);
        Some(Change::Updated(outage.clone()))
    }

    /// Each watched light, by its node or else its ledger row, and its state now.
    #[must_use]
    pub fn connected(&self) -> Vec<Connected> {
        self.lights
            .iter()
            .map(|(key, light)| {
                let state = if light.outage.is_none() {
                    "connected"
                } else {
                    "down"
                };
                (Some(*key), light.device, light.name.clone(), state)
            })
            .chain(
                self.left.iter().map(|(device, outage)| {
                    (None, Some(*device), outage.name.clone(), "left_fabric")
                }),
            )
            .collect()
    }

    #[must_use]
    pub fn status(&self) -> Status {
        let wifi: Vec<&Light> = self
            .lights
            .values()
            .filter(|light| light.network == Some("wifi"))
            .collect();
        Status {
            checked_at: self.checked_at,
            server_reachable: self.checked_at.map(|_| self.server.is_none()),
            server_since: self
                .server
                .as_ref()
                .map(|outage| outage.started_at)
                .or(self.server_up_since),
            server_outage: self.server.clone(),
            wifi_all_down: !wifi.is_empty() && wifi.iter().all(|light| light.outage.is_some()),
            lights: self
                .lights
                .iter()
                .map(|(&(node_id, endpoint), light)| LightStatus {
                    node_id: Some(node_id),
                    endpoint: Some(endpoint),
                    device_id: light.device,
                    name: light.name.clone(),
                    network: light.network,
                    connected: light.outage.is_none(),
                    state: if light.outage.is_none() {
                        "connected"
                    } else {
                        "down"
                    },
                    since: light
                        .outage
                        .as_ref()
                        .map(|outage| outage.started_at)
                        .or(light.up_since),
                    last_answer: light.last_answer,
                    outage: light.outage.clone(),
                })
                .chain(self.left.iter().map(|(device, outage)| LightStatus {
                    node_id: None,
                    endpoint: None,
                    device_id: Some(*device),
                    name: outage.name.clone(),
                    network: None,
                    connected: false,
                    state: "left_fabric",
                    since: Some(outage.started_at),
                    last_answer: None,
                    outage: Some(outage.clone()),
                }))
                .collect(),
        }
    }

    /// What of the lights and matterjs-server asks for the user at `now`: matterjs-server out
    /// of reach for [`ASK_AFTER`], a light a check found still down after its
    /// [`RECONNECTS_TO_ADVISE`]-th reconnection, and a light out of the fabric for a reason the
    /// user can act on. A light home-link is still bringing back by itself asks nothing.
    #[must_use]
    pub fn attention(&self, now: i64) -> Vec<Attention> {
        let server = self
            .server
            .as_ref()
            .filter(|outage| now - outage.started_at >= ASK_AFTER)
            .map(|outage| Attention::ServerDown {
                since: outage.started_at,
            });
        let still_down = self.lights.values().filter_map(|light| {
            let outage = light.outage.as_ref()?;
            // A check after that reconnection found it still down; later ones keep it asking.
            let checked_after = light
                .last_reconnect
                .zip(self.checked_at)
                .is_some_and(|(reconnect, checked)| reconnect < checked);
            let asks = outage.reconnects > RECONNECTS_TO_ADVISE
                || (outage.reconnects == RECONNECTS_TO_ADVISE && checked_after);
            asks.then(|| Attention::PowerCycle {
                name: light.name.clone(),
                device_id: light.device,
                since: outage.started_at,
            })
        });
        let left = self.left.iter().filter_map(|(device, outage)| {
            Some(Attention::Left {
                name: outage.name.clone(),
                device_id: *device,
                left_at: outage.left_at.unwrap_or(outage.started_at),
                left_by: outage.left_by,
                reason: outage.rejoin.asks(now)?,
            })
        });
        server.into_iter().chain(still_down).chain(left).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-26 00:00 in Tokyo.
    const T0: i64 = 1_792_940_400;

    fn minute(n: i64) -> i64 {
        T0 + n * 60
    }

    fn light(node: u64, name: &str, available: bool, answered: Option<bool>) -> Seen {
        Seen {
            key: (node, 1),
            name: name.into(),
            network: Some("wifi"),
            device: None,
            ignored: false,
            available,
            answered,
        }
    }

    /// Runs one check a minute from `from` to `to` minutes, `seen` giving the lights at each,
    /// and gathers every step.
    fn run(
        watch: &mut Watch,
        from: i64,
        to: i64,
        seen: impl Fn(i64) -> Option<Vec<Seen>>,
    ) -> Vec<(i64, Step)> {
        (from..=to)
            .map(|n| {
                let lights = seen(n);
                (n, watch.step(minute(n), lights.as_deref(), &[]))
            })
            .collect()
    }

    /// The minutes from `from` to `to` at which the watch asks for the user, after a check each.
    fn asked(
        watch: &mut Watch,
        from: i64,
        to: i64,
        seen: impl Fn(i64) -> Option<Vec<Seen>>,
    ) -> Vec<(i64, Vec<Attention>)> {
        (from..=to)
            .filter_map(|n| {
                let lights = seen(n);
                watch.step(minute(n), lights.as_deref(), &[]);
                let attention = watch.attention(minute(n));
                (!attention.is_empty()).then_some((n, attention))
            })
            .collect()
    }

    fn reconnects(steps: &[(i64, Step)], key: Key) -> Vec<i64> {
        steps
            .iter()
            .filter(|(_, step)| step.reconnect.contains(&key))
            .map(|(n, _)| *n)
            .collect()
    }

    #[test]
    fn a_light_down_is_kept_from_when_it_went_until_it_is_back_and_asks_nothing_meanwhile() {
        let mut watch = Watch::default();
        // Down from minute 1 to 11, back at 12: reconnected at 1 and 11.
        let seen = |n| Some(vec![light(1, "玄関", !(1..12).contains(&n), None)]);
        assert_eq!(asked(&mut Watch::default(), 0, 14, seen), vec![]);
        let steps = run(&mut watch, 0, 14, seen);
        let changes: Vec<_> = steps.iter().flat_map(|(_, s)| s.changes.clone()).collect();
        let Some(Change::Down(down)) = changes.first() else {
            panic!("{changes:?}");
        };
        assert_eq!((down.name.as_str(), down.network), ("玄関", Some("wifi")));
        assert_eq!(down.started_at, minute(1));
        assert!(
            matches!(changes.last(), Some(Change::Up(o)) if o.ended_at == Some(minute(12))),
            "{changes:?}"
        );
    }

    #[test]
    fn a_light_back_within_four_minutes_keeps_its_stretch() {
        let mut watch = Watch::default();
        let steps = run(&mut watch, 0, 10, |n| {
            Some(vec![light(1, "玄関", !(1..5).contains(&n), None)])
        });
        assert_eq!(
            reconnects(&steps, (1, 1)),
            vec![1],
            "still asked to reconnect at once"
        );
        let changes: Vec<_> = steps.iter().flat_map(|(_, s)| s.changes.clone()).collect();
        assert!(
            matches!(changes.last(), Some(Change::Up(o)) if o.started_at == minute(1) && o.ended_at == Some(minute(5))),
            "the history keeps the stretch: {changes:?}"
        );
    }

    #[test]
    fn an_available_light_is_reconnected_after_two_unanswered_reads_not_one() {
        let mut watch = Watch::default();
        let key = (1, 1);
        let mut steps = Vec::new();
        for n in 0..=20 {
            let answered = watch.read_due(minute(n), key).then_some(n < 5);
            steps.push((
                n,
                watch.step(minute(n), Some(&[light(1, "廊下", true, answered)]), &[]),
            ));
        }
        // Read at 0 (answered) and 5 (missed); a miss is read again at once, at 6.
        assert_eq!(reconnects(&steps, key)[0], 6, "two misses in a row");
        let downs: Vec<_> = steps
            .iter()
            .flat_map(|(_, s)| s.changes.clone())
            .filter_map(|c| match c {
                Change::Down(o) => Some(o),
                _ => None,
            })
            .collect();
        assert_eq!(downs.len(), 1);
        assert_eq!(downs[0].found_by, Cause::NoAnswer);
        assert_eq!(downs[0].started_at, minute(5), "down from the first miss");

        // One miss alone, answered at the next read, asks nothing.
        let mut watch = Watch::default();
        let mut steps = Vec::new();
        for n in 0..=20 {
            let answered = watch.read_due(minute(n), key).then_some(n != 5);
            steps.push((
                n,
                watch.step(minute(n), Some(&[light(1, "廊下", true, answered)]), &[]),
            ));
        }
        assert_eq!(reconnects(&steps, key), Vec::<i64>::new());
    }

    #[test]
    fn a_light_is_reconnected_at_most_once_in_ten_minutes_even_when_all_go_down() {
        let mut watch = Watch::default();
        let names = [
            "玄関",
            "廊下",
            "階段",
            "寝室",
            "居間",
            "台所",
            "書斎",
            "洗面",
            "物干し",
            "納戸",
        ];
        let steps = run(&mut watch, 0, 60, |n| {
            Some(
                names
                    .iter()
                    .zip(1..)
                    .map(|(name, node)| light(node, name, n < 2, None))
                    .collect(),
            )
        });
        for (n, step) in &steps {
            assert!(
                step.reconnect.len() <= MAX_RECONNECTS_PER_CHECK,
                "minute {n}: {:?}",
                step.reconnect
            );
        }
        for node in 1..=10 {
            let at = reconnects(&steps, (node, 1));
            assert!(at[0] <= 2 + 3, "light {node} waits its turn: {at:?}");
            assert!(
                at.windows(2).all(|w| w[1] - w[0] >= 10),
                "light {node}: {at:?}"
            );
            assert!(
                at.len() >= 5,
                "light {node} keeps being reconnected: {at:?}"
            );
        }
    }

    #[test]
    fn a_light_matterjs_server_stops_serving_stays_as_it_was() {
        let mut watch = Watch::default();
        let both = |first: bool| vec![light(1, "玄関", first, None), light(2, "廊下", true, None)];
        watch.step(minute(0), Some(&both(true)), &[]);
        watch.step(minute(1), Some(&both(false)), &[]);
        // The first light goes missing from the nodes while down: still down.
        let steps = run(&mut watch, 2, 30, |_| {
            Some(vec![light(2, "廊下", true, None)])
        });
        assert!(
            steps
                .iter()
                .all(|(_, step)| !step.changes.iter().any(|c| matches!(c, Change::Up(_)))),
            "its stretch stays open"
        );
        assert_eq!(reconnects(&steps, (1, 1)), Vec::<i64>::new());
        assert_eq!(
            watch.connected(),
            vec![
                (Some((1, 1)), None, "玄関".into(), "down"),
                (Some((2, 1)), None, "廊下".into(), "connected")
            ]
        );
        // A light up when it goes missing is no longer watched.
        watch.step(minute(31), Some(&[light(1, "玄関", true, None)]), &[]);
        assert_eq!(
            watch.connected(),
            vec![(Some((1, 1)), None, "玄関".into(), "connected")]
        );
    }

    #[test]
    fn stretches_open_when_home_link_stopped_carry_on_after_it_starts() {
        let mut before = Watch::default();
        let mut open = Vec::new();
        for n in 0..=10 {
            let step = before.step(minute(n), Some(&[light(1, "玄関", n < 1, None)]), &[]);
            open.extend(step.changes.into_iter().filter_map(|c| match c {
                Change::Down(o) | Change::Left(o) | Change::Updated(o) => Some(o),
                Change::Up(_) => None,
            }));
        }
        let last = open.last().cloned().expect("the stretch was kept");

        let mut after = Watch::default();
        after.resume(vec![last]);
        let steps = run(&mut after, 20, 25, |n| {
            Some(vec![light(1, "玄関", n >= 24, None)])
        });
        let changes: Vec<_> = steps.iter().flat_map(|(_, s)| s.changes.clone()).collect();
        assert!(
            !changes.iter().any(|c| matches!(c, Change::Down(_))),
            "the same stretch: {changes:?}"
        );
        assert!(
            matches!(changes.last(), Some(Change::Up(o)) if o.started_at == minute(1) && o.ended_at == Some(minute(24))),
            "{changes:?}"
        );
    }

    #[test]
    fn a_light_still_down_after_three_reconnects_asks_to_be_switched_off_and_on() {
        let mut watch = Watch::default();
        let key = (1, 1);
        let mut asked = Vec::new();
        let mut pings = Vec::new();
        for n in 0..=40 {
            let step = watch.step(minute(n), Some(&[light(1, "玄関", n < 1, None)]), &[]);
            for &k in &step.ping {
                pings.push(n);
                // The bulb is on the Wi-Fi: Matter alone is cut.
                watch.reached(k, true);
            }
            for &k in &step.reconnect {
                assert!(watch.reconnected(k, "no_answer".into()).is_some());
            }
            if !watch.attention(minute(n)).is_empty() {
                asked.push(n);
            }
        }
        assert!(!pings.is_empty(), "a Wi-Fi light down is pinged");
        // Reconnected at 1, 11 and 21; still down at the next check, and from then on.
        assert_eq!(asked.first(), Some(&22), "{asked:?}");
        assert_eq!(asked.len(), 19, "until it is back: {asked:?}");
        assert_eq!(
            watch.attention(minute(40)),
            [Attention::PowerCycle {
                name: "玄関".into(),
                device_id: None,
                since: minute(1)
            }]
        );
        let status = watch.status();
        let outage = status.lights[0].outage.clone().expect("down");
        assert_eq!(outage.reconnects, 4);
        assert_eq!(outage.reconnect_result.as_deref(), Some("no_answer"));
        assert_eq!(outage.reachable, Some(true));
        assert_eq!(
            watch
                .reached(key, false)
                .map(|c| matches!(c, Change::Updated(_))),
            Some(true)
        );
        watch.step(minute(41), Some(&[light(1, "玄関", true, None)]), &[]);
        assert_eq!(watch.attention(minute(41)), [], "back");
    }

    #[test]
    fn an_ignored_light_is_not_watched() {
        let mut watch = Watch::default();
        let steps = run(&mut watch, 0, 30, |_| {
            Some(vec![Seen {
                ignored: true,
                ..light(1, "物置", false, None)
            }])
        });
        assert_eq!(watch.attention(minute(30)), []);
        assert_eq!(reconnects(&steps, (1, 1)), Vec::<i64>::new());
        assert!(watch.status().lights.is_empty(), "not watched");
    }

    #[test]
    fn matterjs_server_out_of_reach_five_minutes_asks_for_the_user_instead_of_each_light() {
        let seen = |n: i64| {
            (!(2..12).contains(&n))
                .then(|| vec![light(1, "玄関", true, None), light(2, "廊下", true, None)])
        };
        let asked = asked(&mut Watch::default(), 0, 20, seen);
        assert_eq!(
            asked.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            (7..12).collect::<Vec<_>>(),
            "from five minutes until it answers"
        );
        assert_eq!(asked[0].1, [Attention::ServerDown { since: minute(2) }]);
        let mut watch = Watch::default();
        let steps = run(&mut watch, 0, 20, seen);
        let changes: Vec<_> = steps.iter().flat_map(|(_, s)| s.changes.clone()).collect();
        assert!(
            changes.iter().any(|c| matches!(c, Change::Up(o) if o.key().is_none() && o.found_by == Cause::Unreachable)),
            "the history keeps matterjs-server's stretch: {changes:?}"
        );
        assert_eq!(reconnects(&steps, (1, 1)), Vec::<i64>::new());
    }

    #[test]
    fn the_status_tells_matterjs_server_and_the_wifi_lights_apart() {
        let mut watch = Watch::default();
        let thread = |available| Seen {
            network: Some("thread"),
            ..light(3, "寝室", available, None)
        };
        watch.step(
            minute(0),
            Some(&[light(1, "玄関", true, None), thread(true)]),
            &[],
        );
        let status = watch.status();
        assert_eq!(status.server_reachable, Some(true));
        assert!(!status.wifi_all_down);
        assert_eq!(status.lights[0].since, Some(minute(0)));

        watch.step(
            minute(1),
            Some(&[light(1, "玄関", false, None), thread(true)]),
            &[],
        );
        let status = watch.status();
        assert!(status.wifi_all_down, "the one Wi-Fi light is down");
        assert!(!status.lights[0].connected);
        assert_eq!(status.lights[0].since, Some(minute(1)));
        assert!(status.lights[1].connected);
        assert_eq!(
            watch.connected(),
            vec![
                (Some((1, 1)), None, "玄関".into(), "down"),
                (Some((3, 1)), None, "寝室".into(), "connected")
            ]
        );

        watch.step(minute(2), None, &[]);
        let status = watch.status();
        assert_eq!(status.server_reachable, Some(false));
        assert_eq!(status.server_since, Some(minute(2)));
        assert_eq!(status.checked_at, Some(minute(2)));
    }

    #[test]
    fn an_answered_read_is_the_last_answer() {
        let mut watch = Watch::default();
        assert!(
            watch.read_due(minute(0), (1, 1)),
            "a light never read is read"
        );
        watch.step(minute(0), Some(&[light(1, "玄関", true, Some(true))]), &[]);
        assert!(!watch.read_due(minute(4), (1, 1)));
        assert!(watch.read_due(minute(5), (1, 1)));
        assert_eq!(watch.status().lights[0].last_answer, Some(minute(0)));
    }

    const STORAGE: i64 = 19;

    fn ledger_light(node: u64, name: &str, available: bool) -> Seen {
        Seen {
            device: Some(STORAGE),
            network: Some("thread"),
            ..light(node, name, available, None)
        }
    }

    fn storage_left(removed_at: Option<i64>) -> Left {
        Left {
            device: STORAGE,
            name: "物置き1".into(),
            labels: vec!["room".into(), "storage".into()],
            serial: Some("54EF441001301194".into()),
            ignored: false,
            network: Some("thread"),
            software_version: Some("1.0.0.6".into()),
            thread_parent: Some("0x4c00".into()),
            removed_at,
            leaves: Vec::new(),
        }
    }

    fn other() -> Seen {
        Seen {
            network: Some("thread"),
            ..light(52, "物置き2", true, None)
        }
    }

    /// Runs one check a minute from `from` to `to` minutes, `seen` giving the lights served and
    /// the ledger lights not served at each, and gathers every step.
    fn run_left(
        watch: &mut Watch,
        from: i64,
        to: i64,
        seen: impl Fn(i64) -> Option<(Vec<Seen>, Vec<Left>)>,
    ) -> Vec<(i64, Step)> {
        (from..=to)
            .map(|n| {
                let step = match seen(n) {
                    Some((lights, left)) => watch.step(minute(n), Some(&lights), &left),
                    None => watch.step(minute(n), None, &[]),
                };
                (n, step)
            })
            .collect()
    }

    fn changes(steps: &[(i64, Step)]) -> Vec<(i64, Change)> {
        steps
            .iter()
            .flat_map(|(n, step)| step.changes.iter().map(move |c| (*n, c.clone())))
            .collect()
    }

    fn lefts(steps: &[(i64, Step)]) -> Vec<(i64, Outage)> {
        changes(steps)
            .into_iter()
            .filter_map(|(n, c)| match c {
                Change::Left(o) => Some((n, o)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_ledger_light_never_served_has_left_the_fabric_from_the_first_check() {
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        let downs = lefts(&steps);
        assert_eq!(downs.len(), 1, "{downs:?}");
        let (0, outage) = &downs[0] else {
            panic!("left at the first check: {downs:?}");
        };
        assert_eq!(outage.found_by, Cause::Left);
        assert_eq!(outage.device_id, Some(STORAGE));
        assert_eq!(outage.started_at, minute(0));
        assert_eq!(outage.left_at, Some(minute(0)));
        assert_eq!(outage.left_by, Some(LeftBy::Unknown));
        assert_eq!(outage.serial_number.as_deref(), Some("54EF441001301194"));
        assert_eq!(outage.software_version.as_deref(), Some("1.0.0.6"));
        assert_eq!(outage.thread_parent.as_deref(), Some("0x4c00"));
        assert_eq!(outage.network, Some("thread"));

        assert!(
            !changes(&steps)
                .iter()
                .any(|(_, c)| matches!(c, Change::Down(_))),
            "a stretch out of the fabric, not down"
        );
        assert_eq!(
            watch.attention(minute(30)),
            [],
            "nothing heard of it yet: home-link looks for it"
        );
        assert!(
            steps
                .iter()
                .all(|(_, s)| s.reconnect.is_empty() && s.ping.is_empty()),
            "a light with no node is not reconnected"
        );

        let status = watch.status();
        let left = status
            .lights
            .iter()
            .find(|l| l.device_id == Some(STORAGE))
            .expect("the light that left is watched");
        assert_eq!(left.state, "left_fabric");
        assert!(!left.connected);
        assert_eq!(
            (left.node_id, left.endpoint, left.network),
            (None, None, None)
        );
        assert_eq!(left.name, "物置き1");
        assert_eq!(left.since, Some(minute(0)));
        assert!(!status.wifi_all_down);
    }

    #[test]
    fn a_light_whose_node_goes_while_up_has_left_from_when_matterjs_server_removed_it() {
        let mut watch = Watch::default();
        let removed = minute(3) - 20;
        let steps = run_left(&mut watch, 0, 20, |n| {
            Some(if n < 3 {
                (vec![ledger_light(7, "物置き1", true), other()], vec![])
            } else {
                (vec![other()], vec![storage_left(Some(removed))])
            })
        });
        let downs = lefts(&steps);
        assert_eq!(downs.len(), 1, "{downs:?}");
        let (3, outage) = &downs[0] else {
            panic!("{downs:?}");
        };
        assert_eq!(outage.found_by, Cause::Left);
        assert_eq!((outage.node_id, outage.endpoint), (Some(7), Some(1)));
        assert_eq!(outage.started_at, removed);
        assert_eq!(outage.left_by, Some(LeftBy::Event));
        assert!(steps.iter().all(|(_, s)| s.reconnect.is_empty()));
        let status = watch.status();
        assert_eq!(
            status.lights.len(),
            2,
            "not watched twice: {:?}",
            status.lights
        );
        assert_eq!(
            watch.connected(),
            vec![
                (Some((52, 1)), None, "物置き2".into(), "connected"),
                (None, Some(STORAGE), "物置き1".into(), "left_fabric"),
            ]
        );

        // Without the removal sent, from the check that found it gone.
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 20, |n| {
            Some(if n < 3 {
                (vec![ledger_light(7, "物置き1", true)], vec![])
            } else {
                (vec![], vec![storage_left(None)])
            })
        });
        let Some((3, Change::Left(outage))) = changes(&steps).into_iter().next() else {
            panic!("{steps:?}");
        };
        assert_eq!(
            (outage.started_at, outage.left_by),
            (minute(3), Some(LeftBy::Check))
        );
    }

    #[test]
    fn a_light_down_whose_node_goes_keeps_its_stretch_out_of_the_fabric() {
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |n| {
            Some(if n < 10 {
                (vec![ledger_light(7, "物置き1", n < 1), other()], vec![])
            } else {
                (vec![other()], vec![storage_left(None)])
            })
        });
        let all = changes(&steps);
        let downs = all
            .iter()
            .filter(|(_, c)| matches!(c, Change::Down(_)))
            .count();
        assert_eq!(downs, 1, "one stretch: {all:?}");
        assert!(
            !all.iter().any(|(_, c)| matches!(c, Change::Up(_))),
            "{all:?}"
        );
        let left = lefts(&steps);
        assert_eq!(left.len(), 1, "{all:?}");
        assert_eq!(left[0].0, 10);
        assert_eq!(
            left[0].1.started_at,
            minute(1),
            "down since the first minute"
        );
        assert_eq!(left[0].1.left_at, Some(minute(10)));
        let last_reconnect = steps
            .iter()
            .filter(|(_, s)| !s.reconnect.is_empty())
            .map(|(n, _)| *n)
            .max();
        assert!(last_reconnect < Some(10), "{last_reconnect:?}");
        let status = watch.status();
        let light = status
            .lights
            .iter()
            .find(|l| l.device_id == Some(STORAGE))
            .expect("watched");
        let outage = light.outage.clone().expect("left");
        assert_eq!(outage.found_by, Cause::Unavailable);
        assert_eq!(outage.started_at, minute(1));
        assert_eq!(outage.left_at, Some(minute(10)));
        assert_eq!(outage.device_id, Some(STORAGE));
        assert_eq!(light.state, "left_fabric");
    }

    #[test]
    fn a_light_registered_again_is_back_on_its_ledger_row() {
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |n| {
            Some(if n < 20 {
                (vec![other()], vec![storage_left(None)])
            } else {
                // Registered again: a new node, the same ledger row.
                (vec![ledger_light(99, "物置き1", true), other()], vec![])
            })
        });
        let ups: Vec<_> = changes(&steps)
            .into_iter()
            .filter_map(|(n, c)| match c {
                Change::Up(o) => Some((n, o)),
                _ => None,
            })
            .collect();
        assert_eq!(ups.len(), 1, "{ups:?}");
        assert_eq!(ups[0].1.found_by, Cause::Left);
        assert_eq!(
            (
                ups[0].1.name.as_str(),
                ups[0].1.started_at,
                ups[0].1.ended_at
            ),
            ("物置き1", minute(0), Some(minute(20)))
        );
        let status = watch.status();
        assert_eq!(status.lights.len(), 2);
        assert_eq!(status.lights[0].node_id, Some(52));
        assert_eq!(
            (status.lights[1].node_id, status.lights[1].state),
            (Some(99), "connected")
        );

        // Back within two minutes: the history keeps it.
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 10, |n| {
            Some(if n < 2 {
                (vec![], vec![storage_left(None)])
            } else {
                (vec![ledger_light(99, "物置き1", true)], vec![])
            })
        });
        assert!(
            changes(&steps)
                .iter()
                .any(|(n, c)| *n == 2 && matches!(c, Change::Up(_)))
        );
    }

    #[test]
    fn a_light_home_link_registers_again_is_back_on_its_row_however_soon() {
        let mut watch = Watch::default();
        let mut steps = run_left(&mut watch, 0, 1, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        assert_eq!(watch.pursued(), vec![STORAGE], "looked for");
        watch.look(STORAGE, minute(2), true, true, true);
        assert_eq!(watch.due(minute(2)), vec![(STORAGE, None)], "heard: due");
        let change = watch.attempted(STORAGE, minute(3), Ok(()));
        assert!(
            matches!(&change, Some(Change::Updated(o)) if o.rejoin.rejoined && o.rejoin.attempts == 1),
            "{change:?}"
        );
        assert_eq!(watch.due(minute(3)), vec![], "not tried again");
        steps.extend(run_left(&mut watch, 4, 6, |_| {
            Some((vec![ledger_light(99, "物置き1", true), other()], vec![]))
        }));
        let ups: Vec<_> = changes(&steps)
            .into_iter()
            .filter(|(_, c)| matches!(c, Change::Up(_)))
            .collect();
        let [(4, Change::Up(outage))] = ups.as_slice() else {
            panic!("{ups:?}");
        };
        assert_eq!(
            (
                outage.started_at,
                outage.ended_at,
                outage.rejoin.attempted_at
            ),
            (minute(0), Some(minute(4)), Some(minute(3))),
            "from leaving to back, and when home-link registered it again"
        );
        assert_eq!(
            watch.pursued(),
            Vec::<i64>::new(),
            "back: no longer looked for"
        );
    }

    #[test]
    fn a_light_unheard_asks_to_be_switched_off_and_on_once_that_stood_five_minutes() {
        let mut watch = Watch::default();
        let removed = minute(-5 * 24 * 60 + 9 * 60 + 17);
        let left = || Some((vec![other()], vec![storage_left(Some(removed))]));
        for n in 0..=7 {
            run_left(&mut watch, n, n, |_| left());
            // Unheard since it left: no advertisement from the first look.
            watch.look(STORAGE, minute(n) + 30, true, true, false);
        }
        assert_eq!(watch.attention(minute(5) + 29), [], "under five minutes");
        assert_eq!(
            watch.attention(minute(5) + 30),
            [Attention::Left {
                name: "物置き1".into(),
                device_id: STORAGE,
                left_at: removed,
                left_by: Some(LeftBy::Event),
                reason: Reason::NoAdvert
            }],
            "the name, when it left, and why: the user switches it off and on"
        );
        // Heard and registered again: it asks nothing more.
        watch.look(STORAGE, minute(8), true, true, true);
        watch.attempted(STORAGE, minute(8) + 30, Ok(()));
        assert_eq!(watch.attention(minute(9)), []);
    }

    #[test]
    fn a_refused_code_asks_to_register_again_once_it_stood_five_minutes() {
        let mut watch = Watch::default();
        run_left(&mut watch, 0, 6, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        watch.look(STORAGE, minute(7), true, true, true);
        watch.attempted(STORAGE, minute(8), Err(Reason::WrongCode));
        for n in 9..=20 {
            watch.look(STORAGE, minute(n), true, true, true);
            run_left(&mut watch, n, n, |_| {
                Some((vec![other()], vec![storage_left(None)]))
            });
        }
        assert_eq!(watch.attention(minute(13) - 1), [], "under five minutes");
        assert!(
            matches!(
                watch.attention(minute(13)).as_slice(),
                [Attention::Left {
                    reason: Reason::WrongCode,
                    ..
                }]
            ),
            "{:?}",
            watch.attention(minute(13))
        );
    }

    #[test]
    fn a_light_registered_again_that_leaves_before_the_next_check_is_looked_for_again() {
        let mut watch = Watch::default();
        run_left(&mut watch, 0, 2, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        watch.look(STORAGE, minute(2) + 20, true, true, true);
        watch.attempted(STORAGE, minute(2) + 30, Ok(()));
        assert_eq!(watch.pursued(), Vec::<i64>::new(), "registered again");
        run_left(&mut watch, 3, 3, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        assert_eq!(
            watch.pursued(),
            vec![STORAGE],
            "gone again at the check after: looked for again"
        );
    }

    #[test]
    fn tries_slowed_by_a_lost_adapter_ask_nothing_but_show_the_next_try() {
        let mut watch = Watch::default();
        run_left(&mut watch, 0, 6, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        watch.look(STORAGE, minute(7), true, true, true);
        watch.attempted(STORAGE, minute(8), Err(Reason::AdapterLost));
        watch.look(STORAGE, minute(9), true, true, true);
        watch.attempted(STORAGE, minute(10), Err(Reason::AdapterLost));
        run_left(&mut watch, 10, 20, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        assert_eq!(
            watch.attention(minute(20)),
            [],
            "nothing for the user to do: home-link keeps trying"
        );
        let status = watch.status();
        let pursuit = &status.lights[1].outage.as_ref().expect("left").rejoin;
        assert_eq!(
            (pursuit.reason, pursuit.slowed_until),
            (Some(Reason::AdapterLost), Some(minute(40))),
            "the admin screen shows why and when it is tried next"
        );
    }

    #[test]
    fn how_often_a_light_left_is_kept() {
        let mut watch = Watch::default();
        let before = |days: i64| minute(-days * 24 * 60);
        run_left(&mut watch, 0, 5, |_| {
            Some((
                vec![other()],
                vec![Left {
                    leaves: vec![before(5), before(1)],
                    ..storage_left(None)
                }],
            ))
        });
        let status = watch.status();
        let outage = status.lights[1].outage.as_ref().expect("left");
        assert_eq!(
            outage.leaves,
            Leaves {
                count: 3,
                previous_at: Some(before(1))
            },
            "two before this one"
        );
    }

    #[test]
    fn a_reason_with_nothing_for_the_user_to_do_asks_nothing() {
        let mut watch = Watch::default();
        let mut steps = run_left(&mut watch, 0, 6, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        for reason in [Reason::ConnectFailed, Reason::NoBluetooth] {
            let start = steps.last().map_or(0, |(n, _)| n + 1);
            watch.look(STORAGE, minute(start), true, true, true);
            watch.attempted(STORAGE, minute(start), Err(reason));
            steps.extend(run_left(&mut watch, start, start + 10, |_| {
                Some((vec![other()], vec![storage_left(None)]))
            }));
        }
        let last = steps.last().map_or(0, |(n, _)| *n);
        assert_eq!(watch.attention(minute(last)), [], "home-link keeps trying");
    }

    #[test]
    fn the_pursuit_carries_on_after_home_link_starts_again() {
        let mut watch = Watch::default();
        run_left(&mut watch, 0, 1, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        watch.look(STORAGE, minute(2), true, true, true);
        let Some(Change::Updated(outage)) =
            watch.attempted(STORAGE, minute(3), Err(Reason::ConnectFailed))
        else {
            panic!("kept");
        };
        let mut again = Watch::default();
        again.resume(vec![outage]);
        let status = again.status();
        let pursuit = &status.lights[0].outage.as_ref().expect("left").rejoin;
        assert_eq!(
            (pursuit.attempts, pursuit.reason),
            (1, Some(Reason::ConnectFailed)),
            "taken back"
        );
    }

    #[test]
    fn an_ignored_ledger_light_that_left_is_not_watched() {
        let ignored = || Left {
            ignored: true,
            ..storage_left(None)
        };
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |_| {
            Some((vec![other()], vec![ignored()]))
        });
        assert_eq!(changes(&steps), vec![]);
        assert_eq!(watch.status().lights.len(), 1);

        // Ignored after it left: its stretch ends.
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |n| {
            Some((
                vec![other()],
                vec![if n < 10 {
                    storage_left(None)
                } else {
                    ignored()
                }],
            ))
        });
        assert_eq!(lefts(&steps).len(), 1);
        assert!(
            changes(&steps)
                .iter()
                .any(|(n, c)| *n == 10 && matches!(c, Change::Up(_)))
        );
        assert_eq!(watch.status().lights.len(), 1);
    }

    #[test]
    fn nothing_is_judged_of_the_lights_that_left_while_matterjs_server_is_out_of_reach() {
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 20, |n| {
            (n >= 2).then(|| (vec![other()], vec![storage_left(None)]))
        });
        let downs = lefts(&steps);
        assert_eq!(downs.len(), 1);
        assert_eq!(downs[0].0, 2, "found once matterjs-server answered");

        // A light that left stays left while matterjs-server is out of reach.
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |n| {
            (!(10..25).contains(&n)).then(|| (vec![other()], vec![storage_left(None)]))
        });
        assert_eq!(lefts(&steps).len(), 1);
        assert!(
            !changes(&steps)
                .iter()
                .any(|(_, c)| matches!(c, Change::Up(o) if o.found_by == Cause::Left))
        );
    }

    #[test]
    fn a_light_that_left_carries_on_after_home_link_starts_again() {
        let mut before = Watch::default();
        let mut open = Vec::new();
        for (_, step) in run_left(&mut before, 0, 10, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        }) {
            open.extend(step.changes.into_iter().filter_map(|c| match c {
                Change::Down(o) | Change::Left(o) | Change::Updated(o) => Some(o),
                Change::Up(_) => None,
            }));
        }
        let last = open.last().cloned().expect("kept");

        let mut after = Watch::default();
        after.resume(vec![last]);
        assert_eq!(after.status().lights[0].state, "left_fabric");
        let steps = run_left(&mut after, 20, 40, |n| {
            Some(if n < 30 {
                (vec![other()], vec![storage_left(None)])
            } else {
                (vec![ledger_light(99, "物置き1", true), other()], vec![])
            })
        });
        let all = changes(&steps);
        assert!(
            !all.iter()
                .any(|(_, c)| matches!(c, Change::Down(_) | Change::Left(_))),
            "the same stretch: {all:?}"
        );
        assert!(
            matches!(all.last(), Some((30, Change::Up(o))) if o.started_at == minute(0)),
            "{all:?}"
        );
    }
}
