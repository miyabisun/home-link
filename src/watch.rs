//! Keeping watch on each light's connection: when a light counts as down, when to ask
//! matterjs-server to reconnect it, and what to tell the user. Nothing here talks to
//! matterjs-server, Discord or the database; [`Watch::step`] decides from what one check saw.
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
    rejoin::{Leaves, Pursuit, Reason},
    schedule,
};

/// Every light's On/Off is read for real this often, in seconds.
pub const READ_EVERY: i64 = 300;
/// A light down this long, in seconds, is told to the user; so is matterjs-server out of reach.
pub const NOTIFY_AFTER: i64 = 300;
/// At most this many lights are asked to reconnect in one check, so lights that all go down
/// at once, as when matterjs-server or the Thread border router restarts, do not flood it.
pub const MAX_RECONNECTS_PER_CHECK: usize = 3;
/// One light is asked to reconnect at most this often, in seconds.
pub const RECONNECT_EVERY: i64 = 600;
/// Reads in a row a light that stays `available` leaves unanswered before it is down.
pub const MISSES_TO_RECONNECT: u32 = 2;
/// Reconnections after which a light still down is told to be switched off and on at the wall.
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
    /// The ledger ignores it: it is neither watched nor told.
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
    /// The ledger ignores it: it is neither watched nor told.
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
    #[serde(skip)]
    notified: bool,
    #[serde(skip)]
    advised: bool,
    #[serde(skip)]
    left_told: bool,
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
            notified: false,
            advised: false,
            left_told: false,
        }
    }

    /// The stretch as kept, with whether the user was told of it and told to switch the light off and on.
    #[must_use]
    pub const fn told(mut self, notified: bool, advised: bool) -> Self {
        self.notified = notified;
        self.advised = advised;
        self
    }

    /// Whether the user was told of it.
    #[must_use]
    pub const fn notified(&self) -> bool {
        self.notified
    }

    /// Whether the user was told to switch the light off and on.
    #[must_use]
    pub const fn advised(&self) -> bool {
        self.advised
    }

    /// The stretch as kept, with whether the user was told the light left the fabric.
    #[must_use]
    pub const fn told_left(mut self, told: bool) -> Self {
        self.left_told = told;
        self
    }

    /// Whether the user was told the light left the fabric.
    #[must_use]
    pub const fn left_told(&self) -> bool {
        self.left_told
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
    /// A stretch still open was reconnected, pinged or told.
    Updated(Outage),
    /// A stretch ended: back, or no longer watched.
    Up(Outage),
}

/// A light in a notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Told {
    pub name: String,
    pub network: Option<&'static str>,
    pub since: i64,
    /// Until when, for a light that came back.
    pub until: Option<i64>,
    pub reachable: Option<bool>,
    /// For a light that left the fabric: its labels, serial number, and when and how it left.
    pub labels: Vec<String>,
    pub serial_number: Option<String>,
    pub left_at: Option<i64>,
    pub left_by: Option<LeftBy>,
    pub rejoin: Pursuit,
    pub leaves: Leaves,
}

/// A message to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// Lights down for [`NOTIFY_AFTER`].
    Down(Vec<Told>),
    /// Lights still down after [`RECONNECTS_TO_ADVISE`] reconnections: switch them off and on.
    StillDown(Vec<Told>),
    /// Lights told down that came back.
    Up(Vec<Told>),
    /// Ledger lights matterjs-server no longer serves for [`NOTIFY_AFTER`]: home-link keeps
    /// looking for them to register them again.
    Left(Vec<Told>),
    /// Lights that left, registered again and back on their ledger rows.
    Rejoined(Vec<Told>),
    /// Lights that left and are not back, for a reason that stood [`crate::rejoin::TELL_AFTER`].
    NotRejoined(Vec<Told>),
    /// matterjs-server out of reach for [`NOTIFY_AFTER`].
    ServerDown { since: i64 },
    /// matterjs-server, told out of reach, back.
    ServerUp { since: i64, until: i64 },
    /// The admin screen's test of the destination.
    Test,
}

/// What one check calls for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// Lights to ask matterjs-server to reconnect.
    pub reconnect: Vec<Key>,
    /// Lights whose network reach to check, being down on Wi-Fi.
    pub ping: Vec<Key>,
    pub notices: Vec<Notice>,
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
    /// matterjs-server was out of reach. Without a destination (`notify` false) no notice
    /// comes out, and every other decision is the same.
    pub fn step(&mut self, at: i64, seen: Option<&[Seen]>, left: &[Left], notify: bool) -> Step {
        self.checked_at = Some(at);
        let mut step = Step::default();
        match seen {
            None => self.server_down(at, &mut step),
            Some(seen) => {
                self.server_up(at, &mut step);
                self.lights_seen(at, seen, left, &mut step);
            }
        }
        if !notify {
            step.notices.clear();
        }
        step
    }

    fn server_down(&mut self, at: i64, step: &mut Step) {
        self.server_up_since = None;
        let outage = self.server.get_or_insert_with(|| {
            let outage = Outage::new(None, "matterjs-server", None, at, Cause::Unreachable);
            step.changes.push(Change::Down(outage.clone()));
            outage
        });
        if !outage.notified && at - outage.started_at >= NOTIFY_AFTER {
            outage.notified = true;
            step.notices.push(Notice::ServerDown {
                since: outage.started_at,
            });
            step.changes.push(Change::Updated(outage.clone()));
        }
    }

    fn server_up(&mut self, at: i64, step: &mut Step) {
        self.server_up_since.get_or_insert(at);
        if let Some(mut outage) = self.server.take() {
            outage.ended_at = Some(at);
            if outage.notified {
                step.notices.push(Notice::ServerUp {
                    since: outage.started_at,
                    until: at,
                });
            }
            step.changes.push(Change::Up(outage));
        }
    }

    fn lights_seen(&mut self, at: i64, seen: &[Seen], left: &[Left], step: &mut Step) {
        let watched: Vec<&Seen> = seen.iter().filter(|light| !light.ignored).collect();
        let gone: Vec<&Left> = left.iter().filter(|light| !light.ignored).collect();
        let (mut up, mut rejoined) = (Vec::new(), Vec::new());
        self.leave(at, &gone, step);
        self.come_back(at, &watched, &gone, &mut rejoined, step);
        self.forget(at, seen, &watched, step);
        let mut due = Vec::new();
        let (mut down, mut still_down) = (Vec::new(), Vec::new());
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
                    if outage.notified {
                        up.push(told(&outage));
                    }
                    step.changes.push(Change::Up(outage));
                }
                light.up_since.get_or_insert(at);
                continue;
            }
            light.up_since = None;
            let outage = light.outage.get_or_insert_with(|| {
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
            let mut updated = false;
            if !outage.notified && at - outage.started_at >= NOTIFY_AFTER {
                outage.notified = true;
                down.push(told(outage));
                updated = true;
            }
            if outage.notified
                && !outage.advised
                && outage.reconnects >= RECONNECTS_TO_ADVISE
                && light.last_reconnect.is_some_and(|last| last < at)
            {
                outage.advised = true;
                still_down.push(told(outage));
                updated = true;
            }
            if updated {
                step.changes.push(Change::Updated(outage.clone()));
            }
        }
        self.reconnect(at, due, step);
        let (mut left, mut not_rejoined) = (Vec::new(), Vec::new());
        for outage in self.left.values_mut() {
            let since = outage.left_at.unwrap_or(outage.started_at);
            if !outage.left_told && at - since >= NOTIFY_AFTER {
                outage.left_told = true;
                // The reason now goes with the message that it left.
                outage.rejoin.told_now();
                left.push(told(outage));
                step.changes.push(Change::Updated(outage.clone()));
            } else if outage.left_told && outage.rejoin.tell(at).is_some() {
                not_rejoined.push(told(outage));
                step.changes.push(Change::Updated(outage.clone()));
            }
        }
        for (lights, notice) in [
            (down, Notice::Down as fn(Vec<Told>) -> Notice),
            (still_down, Notice::StillDown),
            (left, Notice::Left),
            (not_rejoined, Notice::NotRejoined),
            (up, Notice::Up),
            (rejoined, Notice::Rejoined),
        ] {
            if !lights.is_empty() {
                step.notices.push(notice(lights));
            }
        }
    }

    /// Ledger lights matterjs-server no longer serves have left the fabric: a light watched
    /// keeps its stretch, or starts one, from when matterjs-server removed it, else from this
    /// check; one never seen starts one now. It is watched by its ledger row from then on.
    fn leave(&mut self, at: i64, gone: &[&Left], step: &mut Step) {
        for left in gone {
            if let Some(outage) = self.left.get_mut(&left.device) {
                outage.name.clone_from(&left.name);
                outage.labels.clone_from(&left.labels);
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
            let (mut outage, change): (Outage, fn(Outage) -> Change) =
                match light.and_then(|light| light.outage) {
                    Some(outage) => (outage, Change::Updated),
                    None => (
                        Outage::new(watched, &left.name, network, left_at, Cause::Left),
                        Change::Down,
                    ),
                };
            outage.device_id = Some(left.device);
            outage.left_at = Some(left_at);
            outage.left_by = Some(left_by);
            outage.network = network;
            outage.labels.clone_from(&left.labels);
            outage.serial_number.clone_from(&left.serial);
            outage.software_version.clone_from(&left.software_version);
            outage.thread_parent.clone_from(&left.thread_parent);
            outage.leaves = Leaves::of(&left.leaves, left_at);
            step.changes.push(change(outage.clone()));
            self.left.insert(left.device, outage);
        }
    }

    /// A light that left and is served again, registered again, ends its stretch, told back if
    /// home-link registered it again or it was told gone; one no longer in the ledger as a
    /// light to watch ends it untold.
    fn come_back(
        &mut self,
        at: i64,
        watched: &[&Seen],
        gone: &[&Left],
        up: &mut Vec<Told>,
        step: &mut Step,
    ) {
        let ended: Vec<(i64, bool)> = self
            .left
            .keys()
            .filter(|device| !gone.iter().any(|left| left.device == **device))
            .map(|device| {
                let back = watched.iter().any(|seen| seen.device == Some(*device));
                (*device, back)
            })
            .collect();
        for (device, back) in ended {
            let Some(mut outage) = self.left.remove(&device) else {
                continue;
            };
            outage.ended_at = Some(at);
            if back && (outage.rejoin.rejoined || outage.left_told || outage.notified) {
                up.push(told(&outage));
            }
            step.changes.push(Change::Up(outage));
        }
    }

    /// A light ignored since ends its stretch untold. One matterjs-server no longer serves
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

    /// Takes back the stretches still open when home-link last stopped: what was told stays told.
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
}

fn told(outage: &Outage) -> Told {
    Told {
        name: outage.name.clone(),
        network: outage.network,
        since: outage.started_at,
        until: outage.ended_at,
        reachable: outage.reachable,
        labels: outage.labels.clone(),
        serial_number: outage.serial_number.clone(),
        left_at: outage.left_at,
        left_by: outage.left_by,
        rejoin: outage.rejoin.clone(),
        leaves: outage.leaves,
    }
}

/// How often a light left, when more than this once: `14日で3回目です（前回 10-21 09:17）。`
fn leaves_text(leaves: Leaves) -> Option<String> {
    (leaves.count > 1).then(|| {
        let previous = leaves
            .previous_at
            .map(|at| format!("（前回 {}）", date_time(at)))
            .unwrap_or_default();
        format!("14日で{}回目です{previous}。", leaves.count)
    })
}

/// One sentence or two per light, a line each.
fn sentences(lights: &[Told], line: impl Fn(&Told) -> String) -> String {
    lights.iter().map(line).collect::<Vec<_>>().join("\n")
}

fn network_name(network: Option<&str>) -> &'static str {
    match network {
        Some("wifi") => "（Wi-Fi）",
        Some("thread") => "（Thread）",
        Some("ethernet") => "（有線）",
        _ => "",
    }
}

fn hhmm(unix: i64) -> String {
    schedule::clock(schedule::local(unix).1)
}

/// `unix` as month, day and time in Tokyo: `10-21 09:17`.
fn date_time(unix: i64) -> String {
    let stamp = schedule::timestamp(unix);
    stamp.get(5..16).unwrap_or_default().replace('T', " ")
}

/// `seconds` in minutes, or hours and minutes.
fn span(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    if minutes < 60 {
        format!("{minutes}分")
    } else {
        format!("{}時間{}分", minutes / 60, minutes % 60)
    }
}

impl Notice {
    /// The message as sent, in Japanese.
    #[must_use]
    pub fn text(&self) -> String {
        let lines = |lights: &[Told], line: &dyn Fn(&Told) -> String| {
            lights
                .iter()
                .map(|light| {
                    format!(
                        "- {}{}: {}",
                        light.name,
                        network_name(light.network),
                        line(light)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        match self {
            Self::Down(lights) => format!(
                "照明が{}以上つながっていません。\n{}",
                span(NOTIFY_AFTER),
                lines(lights, &|light| format!("{}から", hhmm(light.since)))
            ),
            Self::StillDown(lights) => format!(
                "照明が、つなぎ直しを{RECONNECTS_TO_ADVISE}回しても戻りません。壁のスイッチで電源を入れ直してください。\n{}",
                lines(lights, &|light| {
                    let reach = match (light.network, light.reachable) {
                        (_, Some(true)) => "、網の層では届く（Matterの通信だけが切れている）",
                        (_, Some(false)) => "、網の層でも届かない（Wi-Fiから外れている）",
                        (Some("wifi"), None) => "、網の層は未確認",
                        _ => "",
                    };
                    format!("{}から{reach}", hhmm(light.since))
                })
            ),
            Self::Up(lights) => format!(
                "照明がつながりました。\n{}",
                lines(lights, &|light| format!(
                    "{}つながっていませんでした",
                    span(light.until.unwrap_or(light.since) - light.since)
                ))
            ),
            Self::ServerDown { since } => format!(
                "matterjs-serverに{}以上届きません（{}から）。照明を操作できません。",
                span(NOTIFY_AFTER),
                hhmm(*since)
            ),
            Self::ServerUp { since, until } => format!(
                "matterjs-serverに再び届きました（{}届きませんでした）。",
                span(until - since)
            ),
            Self::Left(lights) => sentences(lights, |light| {
                let mut text = format!(
                    "{}の登録が外れました（{}）。自動で登録し直しています。",
                    light.name,
                    hhmm(light.left_at.unwrap_or(light.since))
                );
                text.extend(leaves_text(light.leaves));
                if let Some(reason) = light.rejoin.reason {
                    text.push_str(&reason.sentence(&light.name));
                }
                text
            }),
            Self::Rejoined(lights) => sentences(lights, |light| {
                let back = light.until.unwrap_or(light.since);
                format!(
                    "{}を登録し直しました（外れてから{}）。",
                    light.name,
                    span(back - light.left_at.unwrap_or(light.since))
                )
            }),
            Self::NotRejoined(lights) => sentences(lights, |light| {
                light
                    .rejoin
                    .reason
                    .unwrap_or(Reason::NoAdvert)
                    .sentence(&light.name)
            }),
            Self::Test => {
                "home-linkからの試しの知らせです。照明の見張りの知らせはここへ届きます。".into()
            }
        }
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
        notify: bool,
        seen: impl Fn(i64) -> Option<Vec<Seen>>,
    ) -> Vec<(i64, Step)> {
        (from..=to)
            .map(|n| {
                let lights = seen(n);
                (n, watch.step(minute(n), lights.as_deref(), &[], notify))
            })
            .collect()
    }

    fn notices(steps: &[(i64, Step)]) -> Vec<(i64, Notice)> {
        steps
            .iter()
            .flat_map(|(n, step)| step.notices.iter().map(move |notice| (*n, notice.clone())))
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
    fn a_light_down_five_minutes_is_told_once_and_once_more_when_back() {
        let mut watch = Watch::default();
        // Down from minute 1 to 11, back at 12.
        let steps = run(&mut watch, 0, 14, true, |n| {
            Some(vec![light(1, "玄関", !(1..12).contains(&n), None)])
        });
        let told = notices(&steps);
        assert_eq!(told.len(), 2, "one down and one back: {told:?}");
        let (at, Notice::Down(lights)) = &told[0] else {
            panic!("first a down notice: {told:?}");
        };
        assert_eq!(*at, 6, "told five minutes after it went down");
        assert_eq!(lights[0].name, "玄関");
        assert_eq!(lights[0].network, Some("wifi"));
        assert_eq!(lights[0].since, minute(1));
        let (at, Notice::Up(lights)) = &told[1] else {
            panic!("then a back notice: {told:?}");
        };
        assert_eq!(*at, 12);
        assert_eq!(
            (lights[0].since, lights[0].until),
            (minute(1), Some(minute(12)))
        );
        assert!(
            told[1].1.text().contains("11分つながっていませんでした"),
            "{}",
            told[1].1.text()
        );
    }

    #[test]
    fn a_light_back_within_four_minutes_is_not_told() {
        let mut watch = Watch::default();
        let steps = run(&mut watch, 0, 10, true, |n| {
            Some(vec![light(1, "玄関", !(1..5).contains(&n), None)])
        });
        assert_eq!(notices(&steps), vec![]);
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
                watch.step(
                    minute(n),
                    Some(&[light(1, "廊下", true, answered)]),
                    &[],
                    true,
                ),
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
                watch.step(
                    minute(n),
                    Some(&[light(1, "廊下", true, answered)]),
                    &[],
                    true,
                ),
            ));
        }
        assert_eq!(reconnects(&steps, key), Vec::<i64>::new());
        assert_eq!(notices(&steps), vec![]);
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
        let steps = run(&mut watch, 0, 60, true, |n| {
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
        watch.step(minute(0), Some(&both(true)), &[], true);
        watch.step(minute(1), Some(&both(false)), &[], true);
        // The first light goes missing from the nodes while down: still down, untold back.
        let steps = run(&mut watch, 2, 30, true, |_| {
            Some(vec![light(2, "廊下", true, None)])
        });
        let told = notices(&steps);
        assert!(
            told.iter()
                .all(|(_, notice)| !matches!(notice, Notice::Up(_))),
            "{told:?}"
        );
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
        watch.step(minute(31), Some(&[light(1, "玄関", true, None)]), &[], true);
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
            let step = before.step(minute(n), Some(&[light(1, "玄関", n < 1, None)]), &[], true);
            open.extend(step.changes.into_iter().filter_map(|c| match c {
                Change::Down(o) | Change::Updated(o) => Some(o),
                Change::Up(_) => None,
            }));
        }
        let last = open.last().cloned().expect("the stretch was kept");
        assert!(last.notified(), "told before the restart");

        let mut after = Watch::default();
        after.resume(vec![last]);
        let steps = run(&mut after, 20, 25, true, |n| {
            Some(vec![light(1, "玄関", n >= 24, None)])
        });
        let told = notices(&steps);
        assert_eq!(told.len(), 1, "not told down again: {told:?}");
        let (24, Notice::Up(lights)) = &told[0] else {
            panic!("{told:?}");
        };
        assert_eq!(lights[0].since, minute(1));
    }

    #[test]
    fn lights_down_at_the_same_time_are_told_in_one_notice() {
        let mut watch = Watch::default();
        let steps = run(&mut watch, 0, 8, true, |n| {
            Some(vec![
                light(1, "玄関", n < 1, None),
                light(2, "廊下", n < 1, None),
                light(3, "階段", n < 1, None),
                light(4, "寝室", true, None),
            ])
        });
        let told = notices(&steps);
        assert_eq!(told.len(), 1, "{told:?}");
        let Notice::Down(lights) = &told[0].1 else {
            panic!("{told:?}");
        };
        let names: Vec<_> = lights.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["玄関", "廊下", "階段"]);
        let text = told[0].1.text();
        assert!(text.contains("玄関（Wi-Fi）: 00:01から"), "{text}");
    }

    #[test]
    fn a_light_still_down_after_three_reconnects_is_told_to_be_switched_off_and_on_with_its_reach()
    {
        let mut watch = Watch::default();
        let key = (1, 1);
        let mut told = Vec::new();
        let mut pings = Vec::new();
        for n in 0..=40 {
            let step = watch.step(minute(n), Some(&[light(1, "玄関", n < 1, None)]), &[], true);
            for &k in &step.ping {
                pings.push(n);
                // The bulb is on the Wi-Fi: Matter alone is cut.
                watch.reached(k, true);
            }
            for &k in &step.reconnect {
                assert!(watch.reconnected(k, "no_answer".into()).is_some());
            }
            told.extend(step.notices.into_iter().map(|notice| (n, notice)));
        }
        assert!(!pings.is_empty(), "a Wi-Fi light down is pinged");
        assert_eq!(told.len(), 2, "{told:?}");
        assert!(matches!(&told[0], (6, Notice::Down(_))), "{told:?}");
        // Reconnected at 1, 11 and 21; still down at the next check.
        let (22, Notice::StillDown(lights)) = &told[1] else {
            panic!("{told:?}");
        };
        assert_eq!(lights[0].reachable, Some(true));
        let text = told[1].1.text();
        assert!(text.contains("電源を入れ直してください"), "{text}");
        assert!(text.contains("網の層では届く"), "{text}");
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
    }

    #[test]
    fn an_ignored_light_is_neither_watched_nor_told() {
        let mut watch = Watch::default();
        let steps = run(&mut watch, 0, 30, true, |_| {
            Some(vec![Seen {
                ignored: true,
                ..light(1, "物置", false, None)
            }])
        });
        assert_eq!(notices(&steps), vec![]);
        assert_eq!(reconnects(&steps, (1, 1)), Vec::<i64>::new());
        assert!(watch.status().lights.is_empty(), "not watched");
    }

    #[test]
    fn matterjs_server_out_of_reach_five_minutes_is_told_once_instead_of_each_light() {
        let mut watch = Watch::default();
        let steps = run(&mut watch, 0, 20, true, |n| {
            (!(2..12).contains(&n))
                .then(|| vec![light(1, "玄関", true, None), light(2, "廊下", true, None)])
        });
        let told = notices(&steps);
        assert_eq!(
            told,
            vec![
                (7, Notice::ServerDown { since: minute(2) }),
                (
                    12,
                    Notice::ServerUp {
                        since: minute(2),
                        until: minute(12)
                    }
                )
            ]
        );
        let changes: Vec<_> = steps.iter().flat_map(|(_, s)| s.changes.clone()).collect();
        assert!(
            changes.iter().any(|c| matches!(c, Change::Up(o) if o.key().is_none() && o.found_by == Cause::Unreachable)),
            "the history keeps matterjs-server's stretch: {changes:?}"
        );
        assert_eq!(reconnects(&steps, (1, 1)), Vec::<i64>::new());
    }

    #[test]
    fn without_a_destination_nothing_is_told_and_all_else_is_the_same() {
        let scenario = |n: i64| {
            Some(vec![
                light(1, "玄関", !(1..30).contains(&n), None),
                light(2, "廊下", true, None),
            ])
            .filter(|_| !(40..50).contains(&n))
        };
        let mut told = Watch::default();
        let mut untold = Watch::default();
        let with = run(&mut told, 0, 60, true, scenario);
        let without = run(&mut untold, 0, 60, false, scenario);
        assert!(!notices(&with).is_empty());
        assert_eq!(notices(&without), vec![]);
        for ((_, a), (_, b)) in with.iter().zip(&without) {
            assert_eq!(
                (&a.reconnect, &a.ping, &a.changes),
                (&b.reconnect, &b.ping, &b.changes)
            );
        }
        assert_eq!(told.status(), untold.status());
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
            true,
        );
        let status = watch.status();
        assert_eq!(status.server_reachable, Some(true));
        assert!(!status.wifi_all_down);
        assert_eq!(status.lights[0].since, Some(minute(0)));

        watch.step(
            minute(1),
            Some(&[light(1, "玄関", false, None), thread(true)]),
            &[],
            true,
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

        watch.step(minute(2), None, &[], true);
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
        watch.step(
            minute(0),
            Some(&[light(1, "玄関", true, Some(true))]),
            &[],
            true,
        );
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
                    Some((lights, left)) => watch.step(minute(n), Some(&lights), &left, true),
                    None => watch.step(minute(n), None, &[], true),
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

    fn left_notices(steps: &[(i64, Step)]) -> Vec<(i64, Vec<Told>)> {
        notices(steps)
            .into_iter()
            .filter_map(|(n, notice)| match notice {
                Notice::Left(lights) => Some((n, lights)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_ledger_light_never_served_has_left_the_fabric_and_is_told_once_after_five_minutes() {
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        let downs: Vec<_> = changes(&steps)
            .into_iter()
            .filter_map(|(n, c)| match c {
                Change::Down(o) => Some((n, o)),
                _ => None,
            })
            .collect();
        assert_eq!(downs.len(), 1, "{downs:?}");
        let (0, outage) = &downs[0] else {
            panic!("down at the first check: {downs:?}");
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

        let told = left_notices(&steps);
        assert_eq!(told.len(), 1, "{told:?}");
        assert_eq!(told[0].0, 5, "told five minutes after it was found");
        assert_eq!(told[0].1[0].name, "物置き1");
        assert_eq!(notices(&steps).len(), 1, "nothing else is told");
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
    fn the_left_notice_names_the_light_and_when_it_left() {
        let mut watch = Watch::default();
        // matterjs-server removed the node on 10-21 at 09:17 Tokyo time.
        let removed = minute(-5 * 24 * 60 + 9 * 60 + 17);
        let steps = run_left(&mut watch, 0, 1, |_| {
            Some((vec![other()], vec![storage_left(Some(removed))]))
        });
        let told = notices(&steps);
        let (0, notice @ Notice::Left(_)) = &told[0] else {
            panic!("told at once, being gone over five minutes: {told:?}");
        };
        assert_eq!(
            notice.text(),
            "物置き1の登録が外れました（09:17）。自動で登録し直しています。",
            "the name, when, and that it is being brought back"
        );
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 5, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        let text = notices(&steps)[0].1.text();
        assert!(text.contains("（00:00）"), "the time it was found: {text}");
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
        let downs: Vec<_> = changes(&steps)
            .into_iter()
            .filter_map(|(n, c)| match c {
                Change::Down(o) => Some((n, o)),
                _ => None,
            })
            .collect();
        assert_eq!(downs.len(), 1, "{downs:?}");
        let (3, outage) = &downs[0] else {
            panic!("{downs:?}");
        };
        assert_eq!(outage.found_by, Cause::Left);
        assert_eq!((outage.node_id, outage.endpoint), (Some(7), Some(1)));
        assert_eq!(outage.started_at, removed);
        assert_eq!(outage.left_by, Some(LeftBy::Event));
        let told = left_notices(&steps);
        assert_eq!(told.len(), 1, "{told:?}");
        assert_eq!(told[0].0, 8, "five minutes from the removal");
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
        let Some((3, Change::Down(outage))) = changes(&steps).into_iter().next() else {
            panic!("{steps:?}");
        };
        assert_eq!(
            (outage.started_at, outage.left_by),
            (minute(3), Some(LeftBy::Check))
        );
        assert_eq!(left_notices(&steps)[0].0, 8);
    }

    #[test]
    fn a_light_down_whose_node_goes_keeps_its_stretch_and_is_told_it_left() {
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
        let told = notices(&steps);
        assert!(matches!(told[0], (6, Notice::Down(_))), "{told:?}");
        let left = left_notices(&steps);
        assert_eq!(left.len(), 1, "{told:?}");
        assert_eq!(left[0].0, 15);
        assert_eq!(left[0].1[0].since, minute(1), "down since the first minute");
        assert_eq!(left[0].1[0].left_at, Some(minute(10)));
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
    fn a_light_registered_again_is_back_on_its_ledger_row_and_told_once() {
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |n| {
            Some(if n < 20 {
                (vec![other()], vec![storage_left(None)])
            } else {
                // Registered again: a new node, the same ledger row.
                (vec![ledger_light(99, "物置き1", true), other()], vec![])
            })
        });
        let told = notices(&steps);
        assert_eq!(told.len(), 2, "{told:?}");
        let (20, Notice::Rejoined(lights)) = &told[1] else {
            panic!("{told:?}");
        };
        assert_eq!(
            (lights[0].name.as_str(), lights[0].since, lights[0].until),
            ("物置き1", minute(0), Some(minute(20)))
        );
        let ups: Vec<_> = changes(&steps)
            .into_iter()
            .filter_map(|(n, c)| match c {
                Change::Up(o) => Some((n, o)),
                _ => None,
            })
            .collect();
        assert_eq!(ups.len(), 1, "{ups:?}");
        assert_eq!(ups[0].1.found_by, Cause::Left);
        assert_eq!(ups[0].1.ended_at, Some(minute(20)));
        let status = watch.status();
        assert_eq!(status.lights.len(), 2);
        assert_eq!(status.lights[0].node_id, Some(52));
        assert_eq!(
            (status.lights[1].node_id, status.lights[1].state),
            (Some(99), "connected")
        );

        // Back before it was told: the history keeps it, nothing is told.
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 10, |n| {
            Some(if n < 2 {
                (vec![], vec![storage_left(None)])
            } else {
                (vec![ledger_light(99, "物置き1", true)], vec![])
            })
        });
        assert_eq!(notices(&steps), vec![]);
        assert!(
            changes(&steps)
                .iter()
                .any(|(n, c)| *n == 2 && matches!(c, Change::Up(_)))
        );
    }

    fn rejoin_notices(steps: &[(i64, Step)]) -> Vec<(i64, Notice)> {
        notices(steps)
            .into_iter()
            .filter(|(_, notice)| matches!(notice, Notice::Rejoined(_) | Notice::NotRejoined(_)))
            .collect()
    }

    #[test]
    fn a_light_home_link_registers_again_is_told_back_on_its_row_however_soon() {
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
        let told = notices(&steps);
        assert_eq!(
            told.len(),
            1,
            "only the return, under five minutes out: {told:?}"
        );
        let (4, notice @ Notice::Rejoined(lights)) = &told[0] else {
            panic!("{told:?}");
        };
        assert_eq!(
            (lights[0].since, lights[0].until),
            (minute(0), Some(minute(4))),
            "from leaving to back"
        );
        let text = notice.text();
        assert_eq!(
            text, "物置き1を登録し直しました（外れてから4分）。",
            "name and time out"
        );
        assert_eq!(
            watch.pursued(),
            Vec::<i64>::new(),
            "back: no longer looked for"
        );
    }

    #[test]
    fn a_light_registered_again_by_the_app_keeps_its_return_message() {
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 12, |n| {
            Some(if n < 10 {
                (vec![other()], vec![storage_left(None)])
            } else {
                (vec![ledger_light(99, "物置き1", true), other()], vec![])
            })
        });
        let told = rejoin_notices(&steps);
        let [(10, notice)] = told.as_slice() else {
            panic!("{told:?}");
        };
        assert!(
            notice.text().contains("外れてから10分"),
            "told back as well: {}",
            notice.text()
        );
    }

    #[test]
    fn the_left_message_says_home_link_keeps_looking_and_why_it_is_not_back() {
        let mut watch = Watch::default();
        let mut steps = Vec::new();
        for n in 0..=5 {
            steps.extend(run_left(&mut watch, n, n, |_| {
                Some((vec![other()], vec![storage_left(None)]))
            }));
            watch.look(STORAGE, minute(n) + 30, true, true, false);
        }
        let left = left_notices(&steps);
        assert_eq!(left.len(), 1, "{left:?}");
        let text = Notice::Left(left[0].1.clone()).text();
        for part in [
            "物置き1の登録が外れました（00:00）",
            "自動で登録し直しています",
            "壁のスイッチ",
        ] {
            assert!(text.contains(part), "{part}: {text}");
        }
        // The reason went with it, and is not told again on its own.
        let later = run_left(&mut watch, 6, 30, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        assert_eq!(rejoin_notices(&later), vec![], "told already");
    }

    #[test]
    fn each_new_reason_that_stands_five_minutes_is_told_once() {
        let mut watch = Watch::default();
        let mut steps = run_left(&mut watch, 0, 6, |_| {
            Some((vec![other()], vec![storage_left(None)]))
        });
        watch.look(STORAGE, minute(7), true, true, true);
        watch.attempted(STORAGE, minute(8), Err(Reason::WrongCode));
        for n in 9..=20 {
            watch.look(STORAGE, minute(n), true, true, true);
            steps.extend(run_left(&mut watch, n, n, |_| {
                Some((vec![other()], vec![storage_left(None)]))
            }));
        }
        let told = rejoin_notices(&steps);
        let [(13, notice @ Notice::NotRejoined(lights))] = told.as_slice() else {
            panic!("five minutes after the refusal: {told:?}");
        };
        assert_eq!(
            lights[0].rejoin.reason,
            Some(Reason::WrongCode),
            "the reason"
        );
        let text = notice.text();
        assert!(
            text.contains("アプリで登録し直してください"),
            "what to do: {text}"
        );
    }

    #[test]
    fn how_often_a_light_left_is_kept_and_told() {
        let mut watch = Watch::default();
        let before = |days: i64| minute(-days * 24 * 60);
        let steps = run_left(&mut watch, 0, 5, |_| {
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
        let text = Notice::Left(left_notices(&steps)[0].1.clone()).text();
        assert!(
            text.contains("14日で3回目です（前回 10-25 00:00）"),
            "{text}"
        );
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
    fn an_ignored_ledger_light_that_left_is_neither_watched_nor_told() {
        let ignored = || Left {
            ignored: true,
            ..storage_left(None)
        };
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |_| {
            Some((vec![other()], vec![ignored()]))
        });
        assert_eq!(notices(&steps), vec![]);
        assert_eq!(changes(&steps), vec![]);
        assert_eq!(watch.status().lights.len(), 1);

        // Ignored after it left: its stretch ends, untold.
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
        assert_eq!(left_notices(&steps).len(), 1);
        assert!(
            !notices(&steps)
                .iter()
                .any(|(_, n)| matches!(n, Notice::Up(_))),
            "not told back"
        );
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
        let downs: Vec<_> = changes(&steps)
            .into_iter()
            .filter(|(_, c)| matches!(c, Change::Down(o) if o.found_by == Cause::Left))
            .collect();
        assert_eq!(downs.len(), 1);
        assert_eq!(downs[0].0, 2, "found once matterjs-server answered");

        // A light that left stays left while matterjs-server is out of reach, and is not told back.
        let mut watch = Watch::default();
        let steps = run_left(&mut watch, 0, 30, |n| {
            (!(10..25).contains(&n)).then(|| (vec![other()], vec![storage_left(None)]))
        });
        assert_eq!(left_notices(&steps).len(), 1);
        assert!(
            !changes(&steps)
                .iter()
                .any(|(_, c)| matches!(c, Change::Up(o) if o.found_by == Cause::Left))
        );
        assert!(
            !notices(&steps)
                .iter()
                .any(|(_, n)| matches!(n, Notice::Up(_)))
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
                Change::Down(o) | Change::Updated(o) => Some(o),
                Change::Up(_) => None,
            }));
        }
        let last = open.last().cloned().expect("kept");
        assert!(last.left_told(), "told before the restart");

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
        let told = notices(&steps);
        assert_eq!(told.len(), 1, "not told again that it left: {told:?}");
        let (30, Notice::Rejoined(lights)) = &told[0] else {
            panic!("{told:?}");
        };
        assert_eq!(lights[0].since, minute(0));
        assert!(
            !changes(&steps)
                .iter()
                .any(|(_, c)| matches!(c, Change::Down(_))),
            "the same stretch"
        );
    }
}
