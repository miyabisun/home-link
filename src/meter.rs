//! The room's `SwitchBot` CO2 meter (W4900010), read from its Bluetooth advertisements
//! through `BlueZ` on the system D-Bus without connecting to it.
//! The bytes follow pySwitchbot's `adv_parsers/meter.py` (`process_wosensorth_c`).

pub mod care;

use care::{Adapter, Care, Mend, Sight, Turn};

use std::{collections::HashMap, sync::Mutex, time::Duration};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use tokio::sync::watch;
use zbus::{
    Connection, MatchRule, MessageStream, Proxy,
    fdo::PropertiesProxy,
    message::Type,
    names::InterfaceName,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

/// The meter's model byte in its `fd3d` service data: `'5'`.
const MODEL: u8 = b'5';
/// How long a reading stands for the room; the meter advertises every few seconds.
pub const FRESH_FOR: i64 = 300;
/// The service UUID `SwitchBot` advertises under.
const SERVICE: &str = "0000fd3d-0000-1000-8000-00805f9b34fb";
/// `SwitchBot`'s Bluetooth company ID.
const SWITCHBOT: u16 = 0x0969;
const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
/// How often discovery is checked, besides at the start and end of each commissioning.
const TICK: Duration = Duration::from_secs(10);
/// How long to wait before reaching the system bus again.
const RETRY: Duration = Duration::from_mins(1);
/// How long connecting to the system bus, and each call to `BlueZ`, may take: zbus waits forever
/// by default, and a stuck `BlueZ` would hold the meter's reading still without a word.
const DBUS_TIMEOUT: Duration = Duration::from_secs(5);
/// How long discovery may stay quiet before it is started again (another HCI user may have stopped it).
pub const QUIET_FOR: i64 = 120;
/// How long a power cycle leaves the adapter off; on homeserver it came back powered on 2-3 s after.
const CYCLE_PAUSE: Duration = Duration::from_secs(3);

/// One advertisement's values.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Reading {
    /// Degrees Celsius to a tenth.
    pub temperature: f64,
    /// Percent.
    pub humidity: u8,
    /// Parts per million.
    pub co2: u16,
    /// Percent.
    pub battery: u8,
}

/// Reads the `fd3d` service data and the manufacturer data of `SwitchBot` (0x0969, without
/// the company ID); `None` unless both come from a CO2 meter.
#[must_use]
pub fn parse(service: &[u8], manufacturer: &[u8]) -> Option<Reading> {
    let (&[model, _, battery, ..], &[.., t0, t1, t2, _, _, c0, c1]) =
        (service, manufacturer.get(..15)?)
    else {
        return None;
    };
    if model & 0x7f != MODEL {
        return None;
    }
    // Bit 7 of the whole degrees set means above zero.
    let tenths = f64::from(u16::from(t1 & 0x7f) * 10 + u16::from(t0 & 0x0f));
    Some(Reading {
        temperature: if t1 & 0x80 == 0 { -tenths } else { tenths } / 10.0,
        // Bit 7 is the display's Fahrenheit switch.
        humidity: t2 & 0x7f,
        co2: u16::from_be_bytes([c0, c1]),
        battery: battery & 0x7f,
    })
}

/// `AA:BB:CC:DD:EE:FF` in upper case, or `None` when `text` is not a Bluetooth address.
#[must_use]
pub fn address(text: &str) -> Option<String> {
    let text = text.trim();
    let well_formed = text.len() == 17
        && text.split(':').count() == 6
        && text
            .split(':')
            .all(|pair| pair.len() == 2 && pair.bytes().all(|b| b.is_ascii_hexdigit()));
    well_formed.then(|| text.to_ascii_uppercase())
}

/// What the receiver does with discovery next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Start,
    Stop,
    Restart,
    Wait,
}

/// Leaves the adapter to a commissioning while one runs (`busy`), and otherwise keeps
/// discovery on once the adapter is `powered`, starting it again after `quiet` seconds
/// without the meter.
#[must_use]
pub(crate) fn step(busy: bool, powered: bool, discovering: bool, quiet: i64) -> Step {
    if !powered && !busy {
        return Step::Wait;
    }
    match (busy, discovering) {
        (true, true) => Step::Stop,
        (false, false) => Step::Start,
        (false, true) if quiet > QUIET_FOR => Step::Restart,
        (true, false) | (false, true) => Step::Wait,
    }
}

/// The configured meter and its latest reading, kept in memory only.
pub(crate) struct Meter {
    address: Option<String>,
    latest: Mutex<Option<(Reading, i64)>>,
    /// How many commissionings are using Bluetooth now.
    busy: watch::Sender<usize>,
    care: Mutex<Care>,
}

/// Marks a commissioning as using Bluetooth until dropped.
pub(crate) struct Hold<'a>(&'a watch::Sender<usize>);

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n -= 1);
    }
}

impl Meter {
    #[must_use]
    pub(crate) fn new(address: Option<String>) -> Self {
        Self {
            address,
            latest: Mutex::new(None),
            busy: watch::Sender::new(0),
            care: Mutex::new(Care::default()),
        }
    }

    #[must_use]
    pub(crate) fn address(&self) -> Option<&str> {
        self.address.as_deref()
    }

    /// Keeps `reading`, received at `at` Unix seconds.
    pub(crate) fn record(&self, reading: Reading, at: i64) {
        *self.latest.lock().expect(crate::UNPOISONED) = Some((reading, at));
    }

    /// Keeps `reading` as heard at `at`, with what it calls for when `tell`.
    fn heard(&self, reading: Reading, at: i64, tell: bool) -> Vec<care::Notice> {
        self.record(reading, at);
        self.care
            .lock()
            .expect(crate::UNPOISONED)
            .heard(at, reading.battery, tell)
    }

    fn look(&self, now: i64, held: bool, sight: Sight, tell: bool) -> Turn {
        self.care
            .lock()
            .expect(crate::UNPOISONED)
            .look(now, held, sight, tell)
    }

    fn latest(&self) -> Option<(Reading, i64)> {
        *self.latest.lock().expect(crate::UNPOISONED)
    }

    /// The latest reading unless it is older than [`FRESH_FOR`] at `now`.
    #[must_use]
    pub(crate) fn reading(&self, now: i64) -> Option<Reading> {
        self.latest()
            .filter(|&(_, at)| now - at <= FRESH_FOR)
            .map(|(reading, _)| reading)
    }

    /// What `GET /api/meter` returns at `now`: the stale reading is `null`, its time stays;
    /// with the adapter as last seen and when it was last powered on or power cycled.
    #[must_use]
    pub(crate) fn view(&self, now: i64) -> serde_json::Value {
        let mut view = json!({
            "address": self.address,
            "reading": self.reading(now),
            "received_at": self.latest().map(|(_, at)| at),
            "fresh_for": FRESH_FOR,
        });
        if let (Some(view), Ok(serde_json::Value::Object(care))) = (
            view.as_object_mut(),
            serde_json::to_value(&*self.care.lock().expect(crate::UNPOISONED)),
        ) {
            view.extend(care);
        }
        view
    }

    /// Takes Bluetooth for a commissioning; the receiver stops discovery until it is dropped.
    pub(crate) fn hold(&self) -> Hold<'_> {
        self.busy.send_modify(|n| *n += 1);
        Hold(&self.busy)
    }

    #[must_use]
    pub(crate) fn watch_busy(&self) -> watch::Receiver<usize> {
        self.busy.subscribe()
    }

    fn held(&self) -> bool {
        *self.busy.borrow() > 0
    }
}

/// Listens to the configured meter forever; without one it returns at once.
/// A missing bus, or one that goes away, is tried again a minute later. Messages for the user
/// go to `deliver` when `tell`.
pub(crate) async fn receive<F, Fut>(meter: &Meter, clock: fn() -> i64, tell: bool, deliver: F)
where
    F: Fn(care::Notice) -> Fut,
    Fut: Future<Output = ()>,
{
    let Some(address) = meter.address() else {
        return;
    };
    let device = format!("/dev_{}", address.replace(':', "_"));
    let mut logged = Logged::default();
    loop {
        if let Err(error) = listen(meter, &device, clock, tell, &deliver, &mut logged).await {
            tracing::warn!(%error, "meter: Bluetooth unavailable, trying again in a minute");
            let sight = Sight::NoBluetooth;
            logged.sight(sight);
            let turn = meter.look(clock(), meter.held(), sight, tell);
            tell_user(&deliver, turn.notice).await;
        }
        tokio::time::sleep(RETRY).await;
    }
}

enum Event {
    Signal(Option<zbus::Result<zbus::Message>>),
    Check,
}

async fn listen<F, Fut>(
    meter: &Meter,
    device: &str,
    clock: fn() -> i64,
    tell: bool,
    deliver: &F,
    logged: &mut Logged,
) -> zbus::Result<()>
where
    F: Fn(care::Notice) -> Fut,
    Fut: Future<Output = ()>,
{
    let bus = tokio::time::timeout(
        DBUS_TIMEOUT,
        zbus::connection::Builder::system()?
            .method_timeout(DBUS_TIMEOUT)
            .build(),
    )
    .await
    .map_err(|_| zbus::Error::Failure("the system bus did not answer in time".into()))??;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path_namespace("/org/bluez")?
        .build();
    let mut signals = MessageStream::for_match_rule(rule, &bus, None).await?;
    let mut busy = meter.watch_busy();
    let mut tick = tokio::time::interval(TICK);
    // Since discovery was last started or the meter last heard, for restarting discovery.
    let mut heard = clock();
    // When the meter was last heard, for the log line once it is heard again.
    let mut last_reading = None;
    loop {
        let event = tokio::select! {
            signal = signals.next() => Event::Signal(signal),
            _ = tick.tick() => Event::Check,
            _ = busy.changed() => Event::Check,
        };
        match event {
            Event::Signal(None) => return Err(zbus::Error::Failure("the bus closed".into())),
            Event::Signal(Some(signal)) => {
                let signal = signal?;
                let Some(path) = signal
                    .header()
                    .path()
                    .filter(|p| p.ends_with(device))
                    .cloned()
                else {
                    continue;
                };
                if let Some(reading) = read(&bus, path.into()).await? {
                    heard = clock();
                    if let Some(quiet) =
                        last_reading.map(|at| heard - at).filter(|&q| q > QUIET_FOR)
                    {
                        tracing::info!(quiet, "meter: heard again");
                    }
                    last_reading = Some(heard);
                    for notice in meter.heard(reading, heard, tell) {
                        tell_user(deliver, Some(notice)).await;
                    }
                }
            }
            Event::Check => {
                let held = *busy.borrow_and_update() > 0;
                let (sight, path) = sense(&bus).await?;
                logged.sight(sight);
                let turn = meter.look(clock(), held, sight, tell);
                tell_user(deliver, turn.notice).await;
                let (Some(path), Sight::Adapter(adapter)) = (path, sight) else {
                    continue;
                };
                if let Some(mend) = turn.mend {
                    logged.mend(mend, path.as_str(), mend_adapter(&bus, &path, mend).await);
                    continue;
                }
                let step = step(held, adapter.powered, adapter.discovering, clock() - heard);
                match tend(&bus, &path, step).await {
                    Ok(started) => {
                        logged.discovery(None);
                        if started {
                            heard = clock();
                        }
                    }
                    // The next look tries again; BlueZ answering, the meter keeps its adapter.
                    Err(error) => logged.discovery(Some(&error)),
                }
            }
        }
    }
}

/// What the log last said of the adapter, so that each change takes one line.
#[derive(Default)]
struct Logged {
    sight: Option<Sight>,
    mend_failed: bool,
    discovery_failed: bool,
}

impl Logged {
    /// The adapter's state as of the latest look.
    fn sight(&mut self, sight: Sight) {
        let state = |sight: Sight| match sight {
            Sight::NoBluetooth => "no_bluetooth",
            Sight::NoAdapter => "no_adapter",
            Sight::Adapter(Adapter { powered: false, .. }) => "powered_off",
            Sight::Adapter(Adapter { powered: true, .. }) => "powered_on",
        };
        if self.sight.is_none_or(|seen| state(seen) != state(sight)) {
            tracing::info!(state = state(sight), "meter: adapter changed");
        }
        self.sight = Some(sight);
    }

    /// Each power on or power cycle done; of those failing in a row, the first.
    fn mend(&mut self, mend: Mend, adapter: &str, done: zbus::Result<()>) {
        let failed = done.is_err();
        match (mend, done) {
            (Mend::PowerOn, Ok(())) => tracing::info!(adapter, "meter: adapter powered on"),
            (Mend::PowerCycle, Ok(())) => tracing::info!(adapter, "meter: adapter power cycled"),
            (Mend::PowerOn, Err(error)) if !self.mend_failed => {
                tracing::warn!(adapter, %error, "meter: adapter not powered on");
            }
            (Mend::PowerCycle, Err(error)) if !self.mend_failed => {
                tracing::warn!(adapter, %error, "meter: adapter not power cycled");
            }
            (_, Err(_)) => {}
        }
        self.mend_failed = failed;
    }

    /// Starting or stopping discovery failing, the first of a row, and working again.
    fn discovery(&mut self, error: Option<&zbus::Error>) {
        match error {
            Some(error) if !self.discovery_failed => {
                tracing::warn!(%error, "meter: discovery not started");
            }
            None if self.discovery_failed => tracing::info!("meter: discovery works again"),
            _ => {}
        }
        self.discovery_failed = error.is_some();
    }
}

async fn tell_user<F, Fut>(deliver: &F, notice: Option<care::Notice>)
where
    F: Fn(care::Notice) -> Fut,
    Fut: Future<Output = ()>,
{
    if let Some(notice) = notice {
        tracing::info!(kind = notice.kind(), "meter: telling the user");
        deliver(notice).await;
    }
}

/// The first adapter and what it is doing, as `BlueZ` says.
async fn sense(bus: &Connection) -> zbus::Result<(Sight, Option<OwnedObjectPath>)> {
    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        Proxy::new(bus, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager")
            .await?
            .call("GetManagedObjects", &())
            .await?;
    let flag = |properties: &HashMap<String, OwnedValue>, name: &str| {
        properties
            .get(name)
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false)
    };
    Ok(objects
        .iter()
        .filter_map(|(path, interfaces)| Some((path, interfaces.get(ADAPTER)?)))
        .min_by_key(|(path, _)| path.as_str())
        .map_or((Sight::NoAdapter, None), |(path, properties)| {
            let adapter = Adapter {
                powered: flag(properties, "Powered"),
                discovering: flag(properties, "Discovering"),
            };
            (Sight::Adapter(adapter), Some(path.clone()))
        }))
}

/// Powers the adapter at `path` on, or off and on again.
async fn mend_adapter(bus: &Connection, path: &OwnedObjectPath, mend: Mend) -> zbus::Result<()> {
    match mend {
        Mend::PowerOn => power(bus, path, true).await,
        Mend::PowerCycle => {
            power(bus, path, false).await?;
            tokio::time::sleep(CYCLE_PAUSE).await;
            power(bus, path, true).await
        }
    }
}

async fn power(bus: &Connection, path: &OwnedObjectPath, on: bool) -> zbus::Result<()> {
    PropertiesProxy::builder(bus)
        .destination(BLUEZ)?
        .path(path.clone())?
        .build()
        .await?
        .set(
            InterfaceName::from_static_str_unchecked(ADAPTER),
            "Powered",
            Value::from(on),
        )
        .await
        .map_err(zbus::Error::from)
}

/// Starts, stops or restarts discovery on the adapter at `path` as `step` says; true once started.
async fn tend(bus: &Connection, path: &OwnedObjectPath, step: Step) -> zbus::Result<bool> {
    let adapter = Proxy::new(bus, BLUEZ, path.as_str(), ADAPTER).await?;
    match step {
        Step::Wait => return Ok(false),
        Step::Stop => {
            // Another client's discovery is not ours to stop; BlueZ refuses and that is fine.
            if adapter.call_method("StopDiscovery", &()).await.is_ok() {
                tracing::info!("meter: discovery stopped for a commissioning");
            }
            return Ok(false);
        }
        Step::Restart => {
            let _ = adapter.call_method("StopDiscovery", &()).await;
        }
        Step::Start => {}
    }
    adapter
        .call_method(
            "SetDiscoveryFilter",
            &(crate::ble_proxy::discovery_filter(),),
        )
        .await?;
    adapter.call_method("StartDiscovery", &()).await?;
    // Restarts while the meter is unheard come every two minutes; the log keeps to changes.
    if step == Step::Restart {
        tracing::debug!(adapter = path.as_str(), "meter: discovery started again");
    } else {
        tracing::info!(adapter = path.as_str(), "meter: discovery started");
    }
    Ok(true)
}

/// Reads the device's last advertisement from `BlueZ`.
async fn read(bus: &Connection, path: OwnedObjectPath) -> zbus::Result<Option<Reading>> {
    let properties = PropertiesProxy::builder(bus)
        .destination(BLUEZ)?
        .path(path)?
        .build()
        .await?
        .get_all(InterfaceName::from_static_str_unchecked(
            "org.bluez.Device1",
        ))
        .await?;
    let service = properties
        .get("ServiceData")
        .and_then(|data| entry(data, &Value::from(SERVICE)));
    let manufacturer = properties
        .get("ManufacturerData")
        .and_then(|data| entry(data, &Value::from(SWITCHBOT)));
    Ok(service
        .zip(manufacturer)
        .and_then(|(service, manufacturer)| parse(&service, &manufacturer)))
}

/// The bytes under `key` in a D-Bus dictionary of byte arrays, looking through variants.
fn entry(dict: &Value, key: &Value) -> Option<Vec<u8>> {
    let Value::Dict(dict) = unwrap(dict) else {
        return None;
    };
    let (_, value) = dict.iter().find(|(k, _)| unwrap(k) == key)?;
    let Value::Array(bytes) = unwrap(value) else {
        return None;
    };
    bytes
        .iter()
        .map(|b| match b {
            Value::U8(b) => Some(*b),
            _ => None,
        })
        .collect()
}

fn unwrap<'a, 'v>(value: &'a Value<'v>) -> &'a Value<'v> {
    match value {
        Value::Value(inner) => unwrap(inner),
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READING: Reading = Reading {
        temperature: 26.7,
        humidity: 57,
        co2: 717,
        battery: 74,
    };

    #[test]
    fn a_reading_stands_for_five_minutes_then_is_withheld() {
        let meter = Meter::new(Some("AA:BB:CC:DD:EE:01".into()));
        assert_eq!(meter.reading(1000), None, "nothing received yet");
        meter.record(READING, 1000);
        assert_eq!(meter.reading(1300), Some(READING), "five minutes old");
        assert_eq!(meter.reading(1301), None, "older than five minutes");
        assert_eq!(
            meter.view(1301),
            json!({
                "address": "AA:BB:CC:DD:EE:01", "reading": null, "received_at": 1000, "fresh_for": 300,
                "adapter": null, "powered_on_at": null, "power_cycled_at": null
            }),
            "the time stays so the screen can say when it stopped"
        );
        assert_eq!(
            meter.view(1000)["reading"],
            json!({"temperature": 26.7, "humidity": 57, "co2": 717, "battery": 74}),
            "a fresh reading"
        );
    }

    #[test]
    fn an_unconfigured_meter_has_nothing() {
        assert_eq!(
            Meter::new(None).view(0),
            json!({
                "address": null, "reading": null, "received_at": null, "fresh_for": 300,
                "adapter": null, "powered_on_at": null, "power_cycled_at": null
            }),
            "no address"
        );
    }

    #[test]
    fn holds_count_commissionings() {
        let meter = Meter::new(None);
        let busy = meter.watch_busy();
        let first = meter.hold();
        let second = meter.hold();
        assert_eq!(*busy.borrow(), 2, "two at once");
        drop(first);
        drop(second);
        assert_eq!(*busy.borrow(), 0, "both done");
    }

    #[test]
    fn discovery_gives_way_to_commissioning() {
        assert_eq!(
            step(true, true, true, 0),
            Step::Stop,
            "commissioning starts"
        );
        assert_eq!(
            step(true, true, false, 999),
            Step::Wait,
            "still commissioning"
        );
        assert_eq!(
            step(false, true, false, 0),
            Step::Start,
            "commissioning done"
        );
        assert_eq!(
            step(false, true, true, QUIET_FOR),
            Step::Wait,
            "heard recently"
        );
        assert_eq!(
            step(false, true, true, QUIET_FOR + 1),
            Step::Restart,
            "discovering but deaf"
        );
    }

    #[test]
    fn discovery_waits_for_the_adapter_to_be_powered() {
        assert_eq!(step(false, false, false, 0), Step::Wait, "powered off");
        assert_eq!(
            step(false, false, true, 999),
            Step::Wait,
            "powered off yet discovering"
        );
        assert_eq!(step(false, true, false, 0), Step::Start, "powered on");
    }

    #[test]
    fn reads_addresses() {
        assert_eq!(
            address(" aa:bb:cc:dd:ee:01 ").as_deref(),
            Some("AA:BB:CC:DD:EE:01"),
            "trimmed and upper case"
        );
        for bad in [
            "",
            "AA:BB:CC:DD:EE",
            "AA-BB-CC-DD-EE-01",
            "AA:BB:CC:DD:EE:0G",
            "AAB:BCC:DD:EE:01:",
        ] {
            assert_eq!(address(bad), None, "{bad}");
        }
    }

    /// A made-up address; the meter repeats its own in the first six bytes.
    const ADDRESS: [u8; 6] = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x01];

    fn manufacturer(rest: [u8; 10]) -> Vec<u8> {
        [ADDRESS.as_slice(), &rest].concat()
    }

    #[test]
    fn reads_the_values_of_2026_10_17() {
        // 26.7 °C, 57 %, 717 ppm, battery 74 %, as received on homeserver.
        let reading = parse(
            &[0x35, 0x00, 0x4a],
            &manufacturer([0x4a, 0x4a, 0x07, 0x9a, 0x39, 0x00, 0x10, 0x02, 0xcd, 0x00]),
        );
        assert_eq!(
            reading,
            Some(Reading {
                temperature: 26.7,
                humidity: 57,
                co2: 717,
                battery: 74
            }),
            "the values the meter showed"
        );
    }

    #[test]
    fn reads_an_advertisement_as_received_on_2026_10_23() {
        let reading = parse(
            &[0x35, 0x00, 0x47],
            &manufacturer([0x47, 0x47, 0x02, 0x9b, 0x30, 0x00, 0x10, 0x02, 0xa8, 0x00]),
        );
        assert_eq!(
            reading,
            Some(Reading {
                temperature: 27.2,
                humidity: 48,
                co2: 680,
                battery: 71
            }),
            "the raw bytes BlueZ reported"
        );
    }

    #[test]
    fn reads_below_zero_and_ignores_the_fahrenheit_display_bit() {
        // The sign bit clear means below zero; the humidity byte's top bit is the display unit.
        let reading = parse(
            &[0x35, 0x00, 0x64],
            &manufacturer([0, 0, 0x05, 0x03, 0xb2, 0, 0, 0x01, 0x90, 0]),
        )
        .expect("a CO2 meter");
        assert!(
            (reading.temperature - -3.5).abs() < 1e-9,
            "{}",
            reading.temperature
        );
        assert_eq!(reading.humidity, 50, "the unit bit is masked off");
        assert_eq!(reading.co2, 400, "big-endian ppm");
        assert_eq!(reading.battery, 100, "battery");
    }

    #[test]
    fn refuses_other_models_and_short_data() {
        let full = manufacturer([0x4a, 0x4a, 0x07, 0x9a, 0x39, 0x00, 0x10, 0x02, 0xcd, 0x00]);
        assert_eq!(
            parse(&[0x69, 0x00, 0x4a], &full),
            None,
            "a Meter Plus has no CO2"
        );
        assert_eq!(parse(&[0x35, 0x00], &full), None, "no battery byte");
        assert_eq!(
            parse(&[0x35, 0x00, 0x4a], &full[..14]),
            None,
            "no CO2 bytes"
        );
    }
}
