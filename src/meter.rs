//! The room's `SwitchBot` CO2 meter (W4900010), read from its Bluetooth advertisements
//! through `BlueZ` on the system D-Bus without connecting to it.
//! The bytes follow pySwitchbot's `adv_parsers/meter.py` (`process_wosensorth_c`).

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
/// How long discovery may stay quiet before it is started again (another HCI user may have stopped it).
pub const QUIET_FOR: i64 = 120;

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
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Start,
    Stop,
    Restart,
    Wait,
}

/// Leaves the adapter to a commissioning while one runs (`busy`), and otherwise keeps
/// discovery on, starting it again after `quiet` seconds without the meter.
#[must_use]
pub(crate) fn step(busy: bool, discovering: bool, quiet: i64) -> Step {
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

    /// What `GET /api/meter` returns at `now`: the stale reading is `null`, its time stays.
    #[must_use]
    pub(crate) fn view(&self, now: i64) -> serde_json::Value {
        json!({
            "address": self.address,
            "reading": self.reading(now),
            "received_at": self.latest().map(|(_, at)| at),
            "fresh_for": FRESH_FOR,
        })
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
}

/// Listens to the configured meter forever; without one it returns at once.
/// A missing bus or adapter, or one that goes away, is tried again a minute later.
pub(crate) async fn receive(meter: &Meter, clock: fn() -> i64) {
    let Some(address) = meter.address() else {
        return;
    };
    let device = format!("/dev_{}", address.replace(':', "_"));
    loop {
        if let Err(error) = listen(meter, &device, clock).await {
            tracing::warn!(%error, "meter: Bluetooth unavailable, trying again in a minute");
        }
        tokio::time::sleep(RETRY).await;
    }
}

enum Event {
    Signal(Option<zbus::Result<zbus::Message>>),
    Check,
}

async fn listen(meter: &Meter, device: &str, clock: fn() -> i64) -> zbus::Result<()> {
    let bus = Connection::system().await?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path_namespace("/org/bluez")?
        .build();
    let mut signals = MessageStream::for_match_rule(rule, &bus, None).await?;
    let mut busy = meter.watch_busy();
    let mut tick = tokio::time::interval(TICK);
    let mut heard = clock();
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
                    meter.record(reading, heard);
                }
            }
            Event::Check => {
                let busy = *busy.borrow_and_update() > 0;
                if tend(&bus, busy, clock() - heard).await? {
                    heard = clock();
                }
            }
        }
    }
}

/// Starts, stops or restarts discovery on the first adapter as [`step`] says; true once started.
async fn tend(bus: &Connection, busy: bool, quiet: i64) -> zbus::Result<bool> {
    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        Proxy::new(bus, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager")
            .await?
            .call("GetManagedObjects", &())
            .await?;
    let Some((path, discovering)) = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let discovering = interfaces.get(ADAPTER)?.get("Discovering")?;
            Some((path, bool::try_from(discovering).unwrap_or(false)))
        })
        .min_by_key(|(path, _)| path.as_str())
    else {
        return Ok(false);
    };
    let adapter = Proxy::new(bus, BLUEZ, path.as_str(), ADAPTER).await?;
    match step(busy, discovering, quiet) {
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
    let filter: HashMap<&str, Value> =
        HashMap::from([("Transport", "le".into()), ("DuplicateData", true.into())]);
    adapter
        .call_method("SetDiscoveryFilter", &(filter,))
        .await?;
    adapter.call_method("StartDiscovery", &()).await?;
    tracing::info!(adapter = path.as_str(), "meter: discovery started");
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
            json!({"address": "AA:BB:CC:DD:EE:01", "reading": null, "received_at": 1000, "fresh_for": 300}),
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
            json!({"address": null, "reading": null, "received_at": null, "fresh_for": 300}),
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
        assert_eq!(step(true, true, 0), Step::Stop, "commissioning starts");
        assert_eq!(step(true, false, 999), Step::Wait, "still commissioning");
        assert_eq!(step(false, false, 0), Step::Start, "commissioning done");
        assert_eq!(step(false, true, QUIET_FOR), Step::Wait, "heard recently");
        assert_eq!(
            step(false, true, QUIET_FOR + 1),
            Step::Restart,
            "discovering but deaf"
        );
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
