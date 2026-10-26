//! Keeping watch on each light's connection: when a light counts as down, when to ask
//! matterjs-server to reconnect it, and what to tell the user. Nothing here talks to
//! matterjs-server, Discord or the database; [`Watch::step`] decides from what one check saw.
//!
//! A check runs every minute on the minute. It reads every node's `available` and, every
//! [`READ_EVERY`], reads each light's On/Off for real, since a light can stay `available`
//! after it stopped answering. A light is down while it is not `available` or after
//! [`MISSES_TO_RECONNECT`] reads in a row went unanswered.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::schedule;

/// Every light's On/Off is read for real this often, in seconds.
pub const READ_EVERY: i64 = 300;
/// A light down this long, in seconds, is told to the user; so is matterjs-server out of reach.
pub const NOTIFY_AFTER: i64 = 300;
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
    /// The ledger ignores it: it is neither watched nor told.
    pub ignored: bool,
    /// matterjs-server serves it as available and, behind a bridge, the bridge reaches it.
    pub available: bool,
    /// Whether it answered a read of its On/Off, when one was sent in this check.
    pub answered: Option<bool>,
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
}

impl Cause {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::NoAnswer => "no_answer",
            Self::Unreachable => "unreachable",
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
    #[serde(skip)]
    notified: bool,
    #[serde(skip)]
    advised: bool,
}

impl Outage {
    fn new(
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
            notified: false,
            advised: false,
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
    server: Option<Outage>,
    server_up_since: Option<i64>,
    checked_at: Option<i64>,
}

/// One light's state for the API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LightStatus {
    pub node_id: u64,
    pub endpoint: u16,
    pub name: String,
    pub network: Option<&'static str>,
    pub connected: bool,
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
    pub fn step(&mut self, at: i64, seen: Option<&[Seen]>, notify: bool) -> Step {
        self.checked_at = Some(at);
        let mut step = Step::default();
        match seen {
            None => self.server_down(at, &mut step),
            Some(seen) => {
                self.server_up(at, &mut step);
                self.lights_seen(at, seen, &mut step);
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

    fn lights_seen(&mut self, at: i64, seen: &[Seen], step: &mut Step) {
        let watched: Vec<&Seen> = seen.iter().filter(|light| !light.ignored).collect();
        // Lights no longer served, or ignored since, end their stretch untold.
        let gone: Vec<Key> = self
            .lights
            .keys()
            .filter(|key| !watched.iter().any(|light| light.key == **key))
            .copied()
            .collect();
        for key in gone {
            if let Some(mut outage) = self.lights.remove(&key).and_then(|light| light.outage) {
                outage.ended_at = Some(at);
                step.changes.push(Change::Up(outage));
            }
        }
        let (mut down, mut still_down, mut up) = (Vec::new(), Vec::new(), Vec::new());
        for seen in watched {
            let light = self.lights.entry(seen.key).or_default();
            light.name.clone_from(&seen.name);
            light.network = seen.network;
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
            let mut updated = false;
            if light
                .last_reconnect
                .is_none_or(|last| at - last >= RECONNECT_EVERY)
            {
                light.last_reconnect = Some(at);
                outage.reconnects += 1;
                step.reconnect.push(seen.key);
                if seen.network == Some("wifi") {
                    step.ping.push(seen.key);
                }
                updated = true;
            }
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
        for (lights, notice) in [
            (down, Notice::Down as fn(Vec<Told>) -> Notice),
            (still_down, Notice::StillDown),
            (up, Notice::Up),
        ] {
            if !lights.is_empty() {
                step.notices.push(notice(lights));
            }
        }
    }

    fn outage_mut(&mut self, key: Key) -> Option<&mut Outage> {
        self.lights.get_mut(&key)?.outage.as_mut()
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

    /// Each watched light and whether it is connected now.
    #[must_use]
    pub fn connected(&self) -> Vec<(Key, String, bool)> {
        self.lights
            .iter()
            .map(|(key, light)| (*key, light.name.clone(), light.outage.is_none()))
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
                    node_id,
                    endpoint,
                    name: light.name.clone(),
                    network: light.network,
                    connected: light.outage.is_none(),
                    since: light
                        .outage
                        .as_ref()
                        .map(|outage| outage.started_at)
                        .or(light.up_since),
                    last_answer: light.last_answer,
                    outage: light.outage.clone(),
                })
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
    }
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
                (n, watch.step(minute(n), lights.as_deref(), notify))
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
                watch.step(minute(n), Some(&[light(1, "廊下", true, answered)]), true),
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
                watch.step(minute(n), Some(&[light(1, "廊下", true, answered)]), true),
            ));
        }
        assert_eq!(reconnects(&steps, key), Vec::<i64>::new());
        assert_eq!(notices(&steps), vec![]);
    }

    #[test]
    fn a_light_is_reconnected_at_most_once_in_ten_minutes_even_when_all_go_down() {
        let mut watch = Watch::default();
        let names = ["玄関", "廊下", "階段", "寝室", "居間"];
        let steps = run(&mut watch, 0, 60, true, |n| {
            Some(
                names
                    .iter()
                    .zip(1..)
                    .map(|(name, node)| light(node, name, n < 2, None))
                    .collect(),
            )
        });
        for node in 1..=5 {
            let at = reconnects(&steps, (node, 1));
            assert_eq!(at, vec![2, 12, 22, 32, 42, 52], "light {node}");
            assert!(
                at.windows(2).all(|w| w[1] - w[0] >= 10),
                "light {node}: {at:?}"
            );
        }
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
            let step = watch.step(minute(n), Some(&[light(1, "玄関", n < 1, None)]), true);
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
        assert!(!watch.read_due(minute(31), (1, 1)) || watch.status().lights.is_empty());
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
            true,
        );
        let status = watch.status();
        assert_eq!(status.server_reachable, Some(true));
        assert!(!status.wifi_all_down);
        assert_eq!(status.lights[0].since, Some(minute(0)));

        watch.step(
            minute(1),
            Some(&[light(1, "玄関", false, None), thread(true)]),
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
                ((1, 1), "玄関".into(), false),
                ((3, 1), "寝室".into(), true)
            ]
        );

        watch.step(minute(2), None, true);
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
        watch.step(minute(0), Some(&[light(1, "玄関", true, Some(true))]), true);
        assert!(!watch.read_due(minute(4), (1, 1)));
        assert!(watch.read_due(minute(5), (1, 1)));
        assert_eq!(watch.status().lights[0].last_answer, Some(minute(0)));
    }
}
