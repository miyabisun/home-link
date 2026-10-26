//! The host's Bluetooth through `BlueZ` on the system D-Bus, and a client of matterjs-server's
//! BLE Proxy Protocol v1 on it, as the phone's app is one on the phone's: matterjs-server runs
//! BTP, PASE and CASE and sends the Bluetooth commands, home-link carries them out.
//!
//! `BlueZ` keeps the adapter, so the meter's discovery and this share it without taking it
//! from each other (nor from the kernel, as the raw HCI channel of noble's proxy did).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{sync::mpsc, time::Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use zbus::{
    Connection, MatchRule, MessageStream, Proxy,
    message::Type,
    zvariant::{OwnedObjectPath, OwnedValue, Value as Variant},
};

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const SERVICE: &str = "org.bluez.GattService1";
const CHARACTERISTIC: &str = "org.bluez.GattCharacteristic1";
/// How long connecting to the system bus, and each call to `BlueZ` but connecting, may take.
const DBUS_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a peripheral may take to connect when matterjs-server names no time.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the handshake with matterjs-server may take.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// The ATT MTU of a link whose `BlueZ` does not say.
const DEFAULT_MTU: u16 = 23;
const PROTOCOL_VERSION: u64 = 1;
/// How long the proxy stays open after its last peripheral went, for matterjs-server to connect again.
const LINK_GRACE: Duration = Duration::from_secs(10);
const WRITE_DATA: u8 = 0x01;
const NOTIFICATION: u8 = 0x02;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The signals of the devices under one adapter.
pub(crate) type Signals = futures_util::stream::BoxStream<'static, zbus::Result<zbus::Message>>;

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// A 16-, 32- or 128-bit UUID in the dashed, lower-case 128-bit form `BlueZ` uses.
#[must_use]
pub(crate) fn canonical(uuid: &str) -> String {
    let hex: String = uuid
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    let full = match hex.len() {
        4 => format!("0000{hex}00001000800000805f9b34fb"),
        8 => format!("{hex}00001000800000805f9b34fb"),
        _ => hex,
    };
    if full.len() != 32 {
        return uuid.to_ascii_lowercase();
    }
    format!(
        "{}-{}-{}-{}-{}",
        &full[..8],
        &full[8..12],
        &full[12..16],
        &full[16..20],
        &full[20..]
    )
}

/// The object path `BlueZ` gives the device at `address` under `adapter`.
#[must_use]
pub(crate) fn device_path(adapter: &str, address: &str) -> String {
    format!(
        "{adapter}/dev_{}",
        address.to_ascii_uppercase().replace(':', "_")
    )
}

/// A binary frame: the opcode, the connection handle big-endian, the payload.
#[must_use]
pub(crate) fn frame(opcode: u8, handle: u16, payload: &[u8]) -> Vec<u8> {
    let [high, low] = handle.to_be_bytes();
    [&[opcode, high, low], payload].concat()
}

/// A binary frame's opcode, handle and payload; `None` when shorter than its header.
#[must_use]
pub(crate) fn unframe(data: &[u8]) -> Option<(u8, u16, &[u8])> {
    let (&[opcode, high, low], payload) = data.split_first_chunk::<3>()?;
    Some((opcode, u16::from_be_bytes([high, low]), payload))
}

/// `BlueZ`'s characteristic flags as the protocol names them.
#[must_use]
pub(crate) fn properties(flags: &[String]) -> Vec<&'static str> {
    flags
        .iter()
        .filter_map(|flag| match flag.as_str() {
            "read" => Some("read"),
            "write" => Some("write"),
            "write-without-response" => Some("writeWithoutResponse"),
            "notify" => Some("notify"),
            "indicate" => Some("indicate"),
            _ => None,
        })
        .collect()
}

/// What `BlueZ` last heard of a device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Heard {
    pub address: String,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    /// Service data by canonical UUID.
    pub service_data: Vec<(String, Vec<u8>)>,
    pub uuids: Vec<String>,
}

impl Heard {
    /// The service data under `uuid`, in any form.
    #[must_use]
    pub fn data(&self, uuid: &str) -> Option<&[u8]> {
        let uuid = canonical(uuid);
        self.service_data
            .iter()
            .find(|(key, _)| *key == uuid)
            .map(|(_, data)| data.as_slice())
    }

    /// Whether a scan for `uuids` (all devices when empty) reports this device.
    #[must_use]
    pub fn wanted(&self, uuids: &[String]) -> bool {
        uuids.is_empty()
            || uuids.iter().map(|uuid| canonical(uuid)).any(|uuid| {
                self.uuids.iter().any(|own| canonical(own) == uuid) || self.data(&uuid).is_some()
            })
    }

    /// The `device_discovered` event.
    #[must_use]
    pub fn event(&self) -> Value {
        let data: serde_json::Map<String, Value> = self
            .service_data
            .iter()
            .map(|(uuid, bytes)| (uuid.clone(), Value::from(encode_base64(bytes))))
            .collect();
        let mut event = json!({
            "event": "device_discovered",
            "data": {
                "address": self.address,
                // BlueZ does not say; a Matter device waiting for commissioning is connectable.
                "connectable": true,
                "service_data": data,
                "service_uuids": self.uuids,
            },
        });
        if let Some(name) = &self.name {
            event["data"]["name"] = name.as_str().into();
        }
        if let Some(rssi) = self.rssi {
            event["data"]["rssi"] = rssi.into();
        }
        event
    }
}

/// A command's answer.
fn answer(id: &Value, result: Result<Value, Refusal>) -> Value {
    match result {
        Ok(result) => json!({ "id": id, "success": true, "result": result }),
        Err(Refusal(code, message)) => {
            json!({ "id": id, "success": false, "error": code, "message": message })
        }
    }
}

/// A command refused, under the protocol's error code.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Refusal(&'static str, String);

fn refused(code: &'static str) -> impl Fn(zbus::Error) -> Refusal {
    move |error| Refusal(code, error.to_string())
}

fn arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, Refusal> {
    args[name]
        .as_str()
        .ok_or_else(|| Refusal("internal_error", format!("missing {name}")))
}

fn handle_of(args: &Value) -> Result<u16, Refusal> {
    args["connection_handle"]
        .as_u64()
        .and_then(|handle| u16::try_from(handle).ok())
        .ok_or_else(|| Refusal("internal_error", "missing connection_handle".into()))
}

fn bytes_of(args: &Value, name: &str) -> Result<Vec<u8>, Refusal> {
    decode_base64(arg(args, name)?)
        .ok_or_else(|| Refusal("internal_error", format!("{name} is not base64")))
}

/// Encodes standard padded base64.
fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let bits = chunk
            .iter()
            .enumerate()
            .fold(0u32, |bits, (i, &b)| bits | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            text.push(if i <= chunk.len() {
                char::from(ALPHABET[(bits >> (18 - 6 * i) & 0x3F) as usize])
            } else {
                '='
            });
        }
    }
    text
}

/// Decodes standard padded base64; `None` on anything else.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let text = text.trim_end_matches('=');
    let mut bits = 0u32;
    let mut count = 0;
    let mut bytes = Vec::new();
    for c in text.bytes() {
        let value = ALPHABET.iter().position(|&a| a == c)?;
        bits = (bits << 6) | u32::try_from(value).ok()?;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push(u8::try_from((bits >> count) & 0xFF).ok()?);
        }
    }
    Some(bytes)
}

fn unwrap<'a, 'v>(value: &'a Variant<'v>) -> &'a Variant<'v> {
    match value {
        Variant::Value(inner) => unwrap(inner),
        value => value,
    }
}

fn byte_array(value: &Variant) -> Option<Vec<u8>> {
    let Variant::Array(bytes) = unwrap(value) else {
        return None;
    };
    bytes
        .iter()
        .map(|b| match unwrap(b) {
            Variant::U8(b) => Some(*b),
            _ => None,
        })
        .collect()
}

fn text(properties: &HashMap<String, OwnedValue>, name: &str) -> Option<String> {
    properties.get(name).and_then(|value| match unwrap(value) {
        Variant::Str(text) => Some(text.to_string()),
        _ => None,
    })
}

fn texts(properties: &HashMap<String, OwnedValue>, name: &str) -> Vec<String> {
    let Some(Variant::Array(items)) = properties.get(name).map(|value| unwrap(value)) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match unwrap(item) {
            Variant::Str(text) => Some(text.to_string()),
            _ => None,
        })
        .collect()
}

fn flag(properties: &HashMap<String, OwnedValue>, name: &str) -> bool {
    properties
        .get(name)
        .is_some_and(|value| matches!(unwrap(value), Variant::Bool(true)))
}

/// A device's properties as heard.
fn heard(properties: &HashMap<String, OwnedValue>) -> Option<Heard> {
    let service_data = match properties.get("ServiceData").map(|value| unwrap(value)) {
        Some(Variant::Dict(dict)) => dict
            .iter()
            .filter_map(|(key, value)| {
                let Variant::Str(key) = unwrap(key) else {
                    return None;
                };
                Some((canonical(key), byte_array(value)?))
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(Heard {
        address: text(properties, "Address")?,
        name: text(properties, "Name"),
        rssi: properties
            .get("RSSI")
            .and_then(|value| match unwrap(value) {
                Variant::I16(rssi) => Some(*rssi),
                _ => None,
            }),
        service_data,
        uuids: texts(properties, "UUIDs"),
    })
}

/// One adapter, through `BlueZ`.
pub(crate) struct Radio {
    bus: Connection,
    adapter: String,
}

impl Radio {
    /// The adapter named `name` (`hci1`), else the first one.
    ///
    /// # Errors
    /// When the system bus or `BlueZ` does not answer, or there is no such adapter.
    pub async fn open(name: Option<&str>) -> zbus::Result<Self> {
        let bus = tokio::time::timeout(
            DBUS_TIMEOUT,
            zbus::connection::Builder::system()?
                .method_timeout(DBUS_TIMEOUT)
                .build(),
        )
        .await
        .map_err(|_| zbus::Error::Failure("the system bus did not answer in time".into()))??;
        let objects = objects(&bus).await?;
        let adapter = objects
            .iter()
            .filter(|(_, interfaces)| interfaces.contains_key(ADAPTER))
            .map(|(path, _)| path.as_str())
            .filter(|path| name.is_none_or(|name| path.rsplit('/').next() == Some(name)))
            .min()
            .ok_or_else(|| {
                zbus::Error::Failure(format!(
                    "no Bluetooth adapter{}",
                    name.map(|name| format!(" {name}")).unwrap_or_default()
                ))
            })?
            .to_owned();
        let radio = Self { bus, adapter };
        if !flag(&radio.adapter_properties().await?, "Powered") {
            return Err(zbus::Error::Failure(format!(
                "{} is powered off",
                radio.name()
            )));
        }
        Ok(radio)
    }

    /// Whether this is the first adapter, the one the meter listens on.
    pub async fn is_first(&self) -> bool {
        objects(&self.bus).await.is_ok_and(|objects| {
            objects
                .iter()
                .filter(|(_, interfaces)| interfaces.contains_key(ADAPTER))
                .map(|(path, _)| path.as_str())
                .min()
                == Some(self.adapter.as_str())
        })
    }

    /// The adapter's name, as `hci0`.
    #[must_use]
    pub fn name(&self) -> &str {
        self.adapter.rsplit('/').next().unwrap_or_default()
    }

    async fn adapter_properties(&self) -> zbus::Result<HashMap<String, OwnedValue>> {
        let proxy = Proxy::new(
            &self.bus,
            BLUEZ,
            self.adapter.as_str(),
            "org.freedesktop.DBus.Properties",
        )
        .await?;
        proxy.call("GetAll", &(ADAPTER,)).await
    }

    async fn adapter(&self) -> zbus::Result<Proxy<'_>> {
        Proxy::new(&self.bus, BLUEZ, self.adapter.as_str(), ADAPTER).await
    }

    /// Keeps discovery on for devices advertising `uuids` (all devices when empty), with every
    /// advertisement reported; `BlueZ` merges this filter with the meter's.
    ///
    /// # Errors
    /// When `BlueZ` refuses the filter or discovery.
    pub async fn scan(&self, uuids: &[String]) -> zbus::Result<()> {
        let uuids: Vec<String> = uuids.iter().map(|uuid| canonical(uuid)).collect();
        let filter: HashMap<&str, Variant> = HashMap::from([
            ("Transport", "le".into()),
            ("DuplicateData", true.into()),
            ("UUIDs", uuids.into()),
        ]);
        let adapter = self.adapter().await?;
        adapter
            .call_method("SetDiscoveryFilter", &(filter,))
            .await?;
        match adapter.call_method("StartDiscovery", &()).await {
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.bluez.Error.InProgress" =>
            {
                Ok(())
            }
            other => other.map(|_| ()),
        }
    }

    /// Ends this client's discovery; `BlueZ` keeps any other client's.
    pub async fn stop_scan(&self) {
        if let Ok(adapter) = self.adapter().await {
            let _ = adapter.call_method("StopDiscovery", &()).await;
        }
    }

    /// The signals of devices under the adapter: their properties changing, their arrival and
    /// their going.
    ///
    /// # Errors
    /// When the bus refuses the match.
    pub async fn signals(&self) -> zbus::Result<Signals> {
        let changed = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")?
            .path_namespace(self.adapter.as_str())?
            .build();
        let objects = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface("org.freedesktop.DBus.ObjectManager")?
            .build();
        let changed = MessageStream::for_match_rule(changed, &self.bus, None).await?;
        let objects = MessageStream::for_match_rule(objects, &self.bus, None).await?;
        Ok(futures_util::stream::select(changed, objects).boxed())
    }

    /// The device a signal is about, if it is one under the adapter.
    #[must_use]
    pub fn device_of(&self, signal: &zbus::Message) -> Option<String> {
        let body = signal.body();
        let path = match signal
            .header()
            .member()
            .map(zbus::names::MemberName::as_str)
        {
            Some("InterfacesAdded") => body
                .deserialize::<(
                    OwnedObjectPath,
                    HashMap<String, HashMap<String, OwnedValue>>,
                )>()
                .ok()?
                .0
                .to_string(),
            Some("InterfacesRemoved") => body
                .deserialize::<(OwnedObjectPath, Vec<String>)>()
                .ok()?
                .0
                .to_string(),
            _ => signal.header().path()?.to_string(),
        };
        let rest = path.strip_prefix(&format!("{}/dev_", self.adapter))?;
        (!rest.contains('/')).then_some(path)
    }

    /// What `BlueZ` last heard of the device at `path`.
    ///
    /// # Errors
    /// When `BlueZ` does not answer.
    pub async fn heard(&self, path: &str) -> zbus::Result<Option<Heard>> {
        let proxy = Proxy::new(&self.bus, BLUEZ, path, "org.freedesktop.DBus.Properties").await?;
        let properties: HashMap<String, OwnedValue> = proxy.call("GetAll", &(DEVICE,)).await?;
        Ok(heard(&properties))
    }

    /// The devices heard under the adapter that `BlueZ` still reports a signal strength for.
    ///
    /// # Errors
    /// When `BlueZ` does not answer.
    pub async fn known(&self) -> zbus::Result<Vec<Heard>> {
        let prefix = format!("{}/dev_", self.adapter);
        Ok(objects(&self.bus)
            .await?
            .iter()
            .filter(|(path, _)| path.as_str().starts_with(&prefix))
            .filter_map(|(_, interfaces)| heard(interfaces.get(DEVICE)?))
            .filter(|heard| heard.rssi.is_some())
            .collect())
    }
}

async fn objects(bus: &Connection) -> zbus::Result<Objects> {
    Proxy::new(bus, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager")
        .await?
        .call("GetManagedObjects", &())
        .await
}

/// One peripheral the proxy holds.
#[derive(Default)]
struct Link {
    path: String,
    /// The characteristic `WRITE_DATA` frames go to.
    write: Option<String>,
    /// The characteristic whose notifications go as binary frames.
    notify: Option<String>,
    /// Characteristics subscribed to, by path, with their UUIDs.
    subscribed: HashMap<String, String>,
}

/// What the proxy holds while it runs: the scan and every peripheral by handle.
#[derive(Default)]
struct Held {
    scanning: Option<Vec<String>>,
    /// Addresses reported in a scan that asked for no duplicates.
    reported: Option<Vec<String>>,
    links: HashMap<u16, Link>,
    next: u16,
    /// Since when no peripheral is connected, once one was.
    unlinked_since: Option<Instant>,
}

impl Held {
    /// Whether the proxy has served its turn at `now`: it connected a peripheral and has held
    /// none for [`LINK_GRACE`]. matterjs-server keeps looking while a proxy is open, even after
    /// the device refused the code; closing it has matterjs-server answer at once.
    fn done(&self, now: Instant) -> bool {
        self.links.is_empty()
            && self
                .unlinked_since
                .is_some_and(|since| now.duration_since(since) >= LINK_GRACE)
    }

    fn link(&mut self, link: Link) -> u16 {
        self.next = self.next.wrapping_add(1).max(1);
        self.links.insert(self.next, link);
        self.unlinked_since = None;
        self.next
    }

    fn unlink(&mut self, handle: u16, now: Instant) -> Option<Link> {
        let link = self.links.remove(&handle)?;
        if self.links.is_empty() {
            self.unlinked_since = Some(now);
        }
        Some(link)
    }
}

/// The proxy's state, shared by the commands, the signals and the end.
type Shared = Arc<Mutex<Held>>;

/// Opens matterjs-server's BLE proxy at `url` and completes the protocol v1 handshake.
async fn greet(url: &str) -> Result<Socket, String> {
    let (mut ws, _) = tokio::time::timeout(HELLO_TIMEOUT, connect_async(url))
        .await
        .map_err(|_| "the BLE proxy did not answer in time".to_owned())?
        .map_err(|error| format!("the BLE proxy refused: {error}"))?;
    let hello = json!({ "type": "hello", "version": PROTOCOL_VERSION });
    ws.send(Message::text(hello.to_string()))
        .await
        .map_err(|error| error.to_string())?;
    let greeted = tokio::time::timeout(HELLO_TIMEOUT, async {
        while let Some(Ok(message)) = ws.next().await {
            if let Message::Text(text) = message
                && let Ok(reply) = serde_json::from_str::<Value>(&text)
                && reply["type"] == "hello_response"
            {
                return reply.get("error").is_none_or(Value::is_null);
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    if !greeted {
        return Err("the BLE proxy did not accept protocol v1".into());
    }
    Ok(ws)
}

/// Serves matterjs-server's BLE proxy at `url` on `radio` until `until` ends, then disconnects
/// every peripheral and ends this client's discovery.
///
/// # Errors
/// When the proxy cannot be opened or greeted, or `BlueZ` cannot be listened to.
pub(crate) async fn serve(
    url: &str,
    radio: &Radio,
    until: impl Future<Output = ()>,
) -> Result<(), String> {
    let (mut sink, mut stream) = greet(url).await?.split();
    let signals = radio.signals().await.map_err(|error| error.to_string())?;
    let held = Shared::default();
    let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
    let (work, mut commands) = mpsc::unbounded_channel::<Message>();
    let write = async {
        while let Some(message) = outgoing.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
    };
    let read = async {
        while let Some(Ok(message)) = stream.next().await {
            if matches!(message, Message::Text(_) | Message::Binary(_))
                && work.send(message).is_err()
            {
                break;
            }
        }
    };
    let run = async {
        while let Some(message) = commands.recv().await {
            match message {
                Message::Text(text) => {
                    let Ok(command) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if command.get("command").is_none() {
                        continue;
                    }
                    let result = execute(radio, &held, &out, &command).await;
                    if let Err(Refusal(code, message)) = &result {
                        tracing::debug!(command = %command["command"], code, message, "BLE proxy command refused");
                    }
                    let _ = out.send(Message::text(answer(&command["id"], result).to_string()));
                }
                Message::Binary(data) => {
                    let Some((WRITE_DATA, handle, payload)) = unframe(&data) else {
                        continue;
                    };
                    let target = held
                        .lock()
                        .expect(crate::UNPOISONED)
                        .links
                        .get(&handle)
                        .and_then(|link| link.write.clone());
                    if let Some(path) = target {
                        // Matter's BTP writes to C1 are acknowledged writes.
                        if let Err(error) = write_value(radio, &path, payload, true).await {
                            tracing::debug!(%error, "BLE proxy write failed");
                        }
                    }
                }
                _ => {}
            }
        }
    };
    let listen = forward(radio, &held, &out, signals);
    let served = async {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if held.lock().expect(crate::UNPOISONED).done(Instant::now()) {
                break;
            }
        }
    };
    tokio::select! {
        () = until => {},
        () = served => {},
        () = write => {},
        () = read => {},
        () = run => {},
        () = listen => {},
    }
    release(radio, &held).await;
    Ok(())
}

/// Disconnects every peripheral the proxy holds and ends its discovery.
async fn release(radio: &Radio, held: &Shared) {
    let (paths, scanning) = {
        let mut held = held.lock().expect(crate::UNPOISONED);
        let paths: Vec<String> = held.links.drain().map(|(_, link)| link.path).collect();
        (paths, held.scanning.take().is_some())
    };
    for path in paths {
        if let Ok(device) = device(radio, &path).await {
            let _ = device.call_method("Disconnect", &()).await;
        }
    }
    if scanning {
        radio.stop_scan().await;
    }
}

async fn device<'a>(radio: &'a Radio, path: &str) -> zbus::Result<Proxy<'a>> {
    Proxy::new(&radio.bus, BLUEZ, path.to_owned(), DEVICE).await
}

async fn characteristic<'a>(radio: &'a Radio, path: &str) -> zbus::Result<Proxy<'a>> {
    Proxy::new(&radio.bus, BLUEZ, path.to_owned(), CHARACTERISTIC).await
}

async fn write_value(radio: &Radio, path: &str, value: &[u8], response: bool) -> zbus::Result<()> {
    let options: HashMap<&str, Variant> =
        HashMap::from([("type", if response { "request" } else { "command" }.into())]);
    characteristic(radio, path)
        .await?
        .call_method("WriteValue", &(value, options))
        .await
        .map(|_| ())
}

/// The GATT objects under `device`, by interface: services or characteristics, with properties.
async fn gatt(
    radio: &Radio,
    under: &str,
    interface: &str,
) -> zbus::Result<Vec<(String, HashMap<String, OwnedValue>)>> {
    let prefix = format!("{under}/");
    let mut found: Vec<(String, HashMap<String, OwnedValue>)> = objects(&radio.bus)
        .await?
        .into_iter()
        .filter(|(path, _)| path.as_str().starts_with(&prefix))
        .filter_map(|(path, mut interfaces)| {
            Some((path.to_string(), interfaces.remove(interface)?))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

/// The characteristic `uuid` of the peripheral at `device`.
async fn find_characteristic(radio: &Radio, device: &str, uuid: &str) -> Result<String, Refusal> {
    let uuid = canonical(uuid);
    gatt(radio, device, CHARACTERISTIC)
        .await
        .map_err(refused("characteristic_not_found"))?
        .into_iter()
        .find(|(_, properties)| text(properties, "UUID").is_some_and(|own| canonical(&own) == uuid))
        .map(|(path, _)| path)
        .ok_or(Refusal("characteristic_not_found", uuid))
}

fn link_path(held: &Shared, handle: u16) -> Result<String, Refusal> {
    held.lock()
        .expect(crate::UNPOISONED)
        .links
        .get(&handle)
        .map(|link| link.path.clone())
        .ok_or_else(|| Refusal("not_connected", format!("no connection {handle}")))
}

/// Waits until `BlueZ` has read the peripheral's GATT database, or `deadline`.
async fn resolved(device: &Proxy<'_>, deadline: Instant) -> Result<(), Refusal> {
    loop {
        if device
            .get_property::<bool>("ServicesResolved")
            .await
            .unwrap_or(false)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Refusal("timeout", "services not resolved".into()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn subscribe(radio: &Radio, held: &Shared, handle: u16, uuid: &str) -> Result<(), Refusal> {
    let device = link_path(held, handle)?;
    let path = find_characteristic(radio, &device, uuid).await?;
    {
        // Notifications may come before StartNotify answers.
        let mut held = held.lock().expect(crate::UNPOISONED);
        if let Some(link) = held.links.get_mut(&handle) {
            link.subscribed.insert(path.clone(), canonical(uuid));
            link.notify = Some(path.clone());
        }
    }
    characteristic(radio, &path)
        .await
        .map_err(refused("subscribe_failed"))?
        .call_method("StartNotify", &())
        .await
        .map_err(refused("subscribe_failed"))?;
    Ok(())
}

async fn write(
    radio: &Radio,
    held: &Shared,
    handle: u16,
    uuid: &str,
    value: &[u8],
    response: bool,
) -> Result<(), Refusal> {
    let device = link_path(held, handle)?;
    let path = find_characteristic(radio, &device, uuid).await?;
    write_value(radio, &path, value, response)
        .await
        .map_err(refused("write_failed"))?;
    if let Some(link) = held.lock().expect(crate::UNPOISONED).links.get_mut(&handle) {
        link.write = Some(path);
    }
    Ok(())
}

/// Carries out one command matterjs-server sent.
async fn execute(
    radio: &Radio,
    held: &Shared,
    out: &mpsc::UnboundedSender<Message>,
    command: &Value,
) -> Result<Value, Refusal> {
    let args = &command["args"];
    match command["command"].as_str().unwrap_or_default() {
        "start_scan" => {
            let uuids: Vec<String> = args["service_uuids"]
                .as_array()
                .map(|uuids| {
                    uuids
                        .iter()
                        .filter_map(Value::as_str)
                        .map(canonical)
                        .collect()
                })
                .unwrap_or_default();
            radio
                .scan(&uuids)
                .await
                .map_err(refused("bluetooth_unavailable"))?;
            let known = radio.known().await.unwrap_or_default();
            {
                let mut held = held.lock().expect(crate::UNPOISONED);
                held.reported =
                    (!args["allow_duplicates"].as_bool().unwrap_or(true)).then(Vec::new);
                held.scanning = Some(uuids);
            }
            for heard in known {
                report(held, out, &heard);
            }
            Ok(json!({}))
        }
        "stop_scan" => {
            if held
                .lock()
                .expect(crate::UNPOISONED)
                .scanning
                .take()
                .is_none()
            {
                return Err(Refusal("not_scanning", "not scanning".into()));
            }
            radio.stop_scan().await;
            Ok(json!({}))
        }
        "connect" => {
            let address = arg(args, "address")?;
            let path = device_path(&radio.adapter, address);
            let timeout = args["timeout"]
                .as_u64()
                .map_or(CONNECT_TIMEOUT, Duration::from_millis);
            let deadline = Instant::now() + timeout;
            let device = device(radio, &path)
                .await
                .map_err(refused("device_not_found"))?;
            tokio::time::timeout_at(deadline, device.call_method("Connect", &()))
                .await
                .map_err(|_| Refusal("timeout", format!("connecting to {address}")))?
                .map_err(refused("connection_failed"))?;
            resolved(&device, deadline).await?;
            let mtu = mtu(radio, &path).await;
            let handle = held.lock().expect(crate::UNPOISONED).link(Link {
                path,
                ..Link::default()
            });
            Ok(json!({ "connection_handle": handle, "mtu": mtu }))
        }
        "disconnect" => {
            let handle = handle_of(args)?;
            let path = link_path(held, handle)?;
            held.lock()
                .expect(crate::UNPOISONED)
                .unlink(handle, Instant::now());
            if let Ok(device) = device(radio, &path).await {
                let _ = device.call_method("Disconnect", &()).await;
            }
            Ok(json!({}))
        }
        name => gatt_command(radio, held, name, args).await,
    }
}

/// Carries out one command on a connected peripheral's GATT database.
async fn gatt_command(
    radio: &Radio,
    held: &Shared,
    name: &str,
    args: &Value,
) -> Result<Value, Refusal> {
    match name {
        "discover_services" => {
            let path = link_path(held, handle_of(args)?)?;
            let services: Vec<Value> = gatt(radio, &path, SERVICE)
                .await
                .map_err(refused("discovery_failed"))?
                .iter()
                .filter_map(|(_, properties)| text(properties, "UUID"))
                .map(|uuid| json!({ "uuid": uuid }))
                .collect();
            Ok(json!({ "services": services }))
        }
        "discover_characteristics" => characteristics(radio, held, args).await,
        "read_characteristic" => {
            let device = link_path(held, handle_of(args)?)?;
            let path =
                find_characteristic(radio, &device, arg(args, "characteristic_uuid")?).await?;
            let options: HashMap<&str, Variant> = HashMap::new();
            let value: Vec<u8> = characteristic(radio, &path)
                .await
                .map_err(refused("read_failed"))?
                .call("ReadValue", &(options,))
                .await
                .map_err(refused("read_failed"))?;
            Ok(json!({ "value": encode_base64(&value) }))
        }
        "write_characteristic" => {
            let handle = handle_of(args)?;
            let value = bytes_of(args, "value")?;
            let response = args["response"].as_bool().unwrap_or(false);
            write(
                radio,
                held,
                handle,
                arg(args, "characteristic_uuid")?,
                &value,
                response,
            )
            .await?;
            Ok(json!({}))
        }
        "subscribe_characteristic" => {
            subscribe(
                radio,
                held,
                handle_of(args)?,
                arg(args, "characteristic_uuid")?,
            )
            .await?;
            Ok(json!({}))
        }
        "write_and_subscribe" => {
            let handle = handle_of(args)?;
            let value = bytes_of(args, "write_value")?;
            let response = args["write_response"].as_bool().unwrap_or(false);
            write(
                radio,
                held,
                handle,
                arg(args, "write_uuid")?,
                &value,
                response,
            )
            .await?;
            subscribe(radio, held, handle, arg(args, "subscribe_uuid")?).await?;
            Ok(json!({}))
        }
        "unsubscribe_characteristic" => {
            let handle = handle_of(args)?;
            let device = link_path(held, handle)?;
            let path =
                find_characteristic(radio, &device, arg(args, "characteristic_uuid")?).await?;
            if let Some(link) = held.lock().expect(crate::UNPOISONED).links.get_mut(&handle) {
                link.subscribed.remove(&path);
                if link.notify.as_deref() == Some(path.as_str()) {
                    link.notify = None;
                }
            }
            let _ = characteristic(radio, &path)
                .await
                .map_err(refused("not_subscribed"))?
                .call_method("StopNotify", &())
                .await;
            Ok(json!({}))
        }
        // BlueZ exchanges the MTU itself on connecting; the one in effect is the answer.
        "request_mtu" => {
            let path = link_path(held, handle_of(args)?)?;
            Ok(json!({ "mtu": mtu(radio, &path).await }))
        }
        other => Err(Refusal(
            "internal_error",
            format!("Unknown command: {other}"),
        )),
    }
}

/// The characteristics of one service of a connected peripheral, with their properties.
async fn characteristics(radio: &Radio, held: &Shared, args: &Value) -> Result<Value, Refusal> {
    let path = link_path(held, handle_of(args)?)?;
    let wanted = canonical(arg(args, "service_uuid")?);
    let services = gatt(radio, &path, SERVICE)
        .await
        .map_err(refused("discovery_failed"))?;
    let service = services
        .iter()
        .find(|(_, properties)| {
            text(properties, "UUID").is_some_and(|uuid| canonical(&uuid) == wanted)
        })
        .map(|(path, _)| path.clone())
        .ok_or(Refusal("service_not_found", wanted))?;
    let characteristics: Vec<Value> = gatt(radio, &service, CHARACTERISTIC)
        .await
        .map_err(refused("discovery_failed"))?
        .iter()
        .filter_map(|(_, properties)| {
            Some(json!({
                "uuid": text(properties, "UUID")?,
                "properties": self::properties(&texts(properties, "Flags")),
            }))
        })
        .collect();
    Ok(json!({ "characteristics": characteristics }))
}

/// The ATT MTU `BlueZ` reports on the peripheral's characteristics, else the least there is.
async fn mtu(radio: &Radio, device: &str) -> u16 {
    gatt(radio, device, CHARACTERISTIC)
        .await
        .unwrap_or_default()
        .iter()
        .find_map(
            |(_, properties)| match properties.get("MTU").map(|value| unwrap(value)) {
                Some(Variant::U16(mtu)) => Some(*mtu),
                _ => None,
            },
        )
        .unwrap_or(DEFAULT_MTU)
}

/// Sends `heard` as discovered when the scan asks for it.
fn report(held: &Shared, out: &mpsc::UnboundedSender<Message>, heard: &Heard) {
    let mut held = held.lock().expect(crate::UNPOISONED);
    let Some(uuids) = &held.scanning else {
        return;
    };
    if !heard.wanted(uuids) {
        return;
    }
    if let Some(reported) = held.reported.as_mut() {
        if reported.contains(&heard.address) {
            return;
        }
        reported.push(heard.address.clone());
    }
    let _ = out.send(Message::text(heard.event().to_string()));
}

/// Forwards what `BlueZ` signals while the proxy runs: devices heard in the scan, notifications
/// of subscribed characteristics, and peripherals that dropped the connection.
async fn forward(
    radio: &Radio,
    held: &Shared,
    out: &mpsc::UnboundedSender<Message>,
    mut signals: Signals,
) {
    while let Some(Ok(signal)) = signals.next().await {
        let header = signal.header();
        if header.member().is_some_and(|m| m == "PropertiesChanged")
            && let Some(path) = header.path().map(ToString::to_string)
            && let Ok((interface, changed, _)) =
                signal
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
        {
            if interface == CHARACTERISTIC {
                let Some(value) = changed.get("Value").and_then(|value| byte_array(value)) else {
                    continue;
                };
                notify(held, out, &path, &value);
                continue;
            }
            if interface == DEVICE
                && changed
                    .get("Connected")
                    .is_some_and(|value| matches!(unwrap(value), Variant::Bool(false)))
            {
                dropped(held, out, &path);
                continue;
            }
        }
        let Some(path) = radio.device_of(&signal) else {
            continue;
        };
        if header.member().is_some_and(|m| m == "InterfacesRemoved") {
            dropped(held, out, &path);
            continue;
        }
        if held.lock().expect(crate::UNPOISONED).scanning.is_none() {
            continue;
        }
        if let Ok(Some(heard)) = radio.heard(&path).await {
            report(held, out, &heard);
        }
    }
}

fn notify(held: &Shared, out: &mpsc::UnboundedSender<Message>, path: &str, value: &[u8]) {
    let held = held.lock().expect(crate::UNPOISONED);
    for (handle, link) in &held.links {
        let Some(uuid) = link.subscribed.get(path) else {
            continue;
        };
        let message = if link.notify.as_deref() == Some(path) {
            Message::binary(frame(NOTIFICATION, *handle, value))
        } else {
            Message::text(
                json!({ "event": "characteristic_notification", "data": {
                    "connection_handle": handle, "characteristic_uuid": uuid,
                    "value": encode_base64(value),
                }})
                .to_string(),
            )
        };
        let _ = out.send(message);
    }
}

fn dropped(held: &Shared, out: &mpsc::UnboundedSender<Message>, path: &str) {
    let mut held = held.lock().expect(crate::UNPOISONED);
    let handles: Vec<u16> = held
        .links
        .iter()
        .filter(|(_, link)| link.path == path)
        .map(|(handle, _)| *handle)
        .collect();
    for handle in handles {
        held.unlink(handle, Instant::now());
        let event = json!({ "event": "disconnected", "data": {
            "connection_handle": handle, "reason": "peripheral disconnected",
        }});
        let _ = out.send(Message::text(event.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_take_bluez_form() {
        let fff6 = "0000fff6-0000-1000-8000-00805f9b34fb";
        for form in [
            "fff6",
            "FFF6",
            "0000fff6",
            "0000FFF600001000800000805F9B34FB",
            fff6,
        ] {
            assert_eq!(canonical(form), fff6, "{form}");
        }
        let c1 = "18ee2ef5-263d-4559-959f-4f9c429f9d11";
        assert_eq!(
            canonical("18EE2EF5263D4559959F4F9C429F9D11"),
            c1,
            "noble's form"
        );
        assert_eq!(canonical("odd"), "odd", "not a UUID stays as it is");
    }

    #[test]
    fn devices_are_found_by_address_under_the_adapter() {
        assert_eq!(
            device_path("/org/bluez/hci1", "aa:bb:cc:dd:ee:01"),
            "/org/bluez/hci1/dev_AA_BB_CC_DD_EE_01",
            "BlueZ's path"
        );
    }

    #[test]
    fn frames_carry_opcode_handle_and_payload() {
        let framed = frame(NOTIFICATION, 0x0102, &[0x65, 0x6c]);
        assert_eq!(framed, [0x02, 0x01, 0x02, 0x65, 0x6c], "big-endian handle");
        assert_eq!(
            unframe(&framed),
            Some((NOTIFICATION, 0x0102, &[0x65, 0x6c][..])),
            "round trip"
        );
        assert_eq!(
            unframe(&[0x01, 0x00, 0x07]),
            Some((WRITE_DATA, 7, &[][..])),
            "empty payload"
        );
        assert_eq!(unframe(&[0x01, 0x00]), None, "too short");
    }

    #[test]
    fn flags_take_the_protocol_names() {
        let flags = ["read", "write-without-response", "indicate", "authorize"].map(String::from);
        assert_eq!(
            properties(&flags),
            ["read", "writeWithoutResponse", "indicate"],
            "BlueZ-only flags dropped"
        );
    }

    fn matter_light() -> Heard {
        Heard {
            address: "AA:BB:CC:DD:EE:01".into(),
            name: None,
            rssi: Some(-61),
            service_data: vec![(
                canonical("fff6"),
                vec![0x00, 0x01, 0x0F, 0xF1, 0xFF, 0x00, 0x80, 0x00],
            )],
            uuids: Vec::new(),
        }
    }

    #[test]
    fn a_scan_reports_devices_with_the_service_by_data_or_uuid() {
        let light = matter_light();
        assert!(light.wanted(&[]), "a scan for everything");
        assert!(
            light.wanted(&["FFF6".into()]),
            "service data under the UUID"
        );
        assert!(!light.wanted(&["fd3d".into()]), "another service");
        let meter = Heard {
            uuids: vec!["0000fd3d-0000-1000-8000-00805f9b34fb".into()],
            ..Heard::default()
        };
        assert!(meter.wanted(&["fd3d".into()]), "an advertised UUID");
        assert_eq!(
            light.data("fff6").map(<[u8]>::len),
            Some(8),
            "found by short UUID"
        );
    }

    #[test]
    fn a_discovered_device_is_sent_as_matterjs_server_reads_it() {
        assert_eq!(
            matter_light().event(),
            json!({ "event": "device_discovered", "data": {
                "address": "AA:BB:CC:DD:EE:01",
                "connectable": true,
                "rssi": -61,
                "service_data": { "0000fff6-0000-1000-8000-00805f9b34fb": "AAEP8f8AgAA=" },
                "service_uuids": [],
            }}),
            "base64 service data by UUID"
        );
    }

    #[test]
    fn answers_carry_the_command_id() {
        assert_eq!(
            answer(&json!(7), Ok(json!({ "mtu": 247 }))),
            json!({ "id": 7, "success": true, "result": { "mtu": 247 } }),
            "success"
        );
        assert_eq!(
            answer(
                &json!(8),
                Err(Refusal("not_connected", "no connection 3".into()))
            ),
            json!({ "id": 8, "success": false, "error": "not_connected", "message": "no connection 3" }),
            "failure"
        );
    }

    #[test]
    fn the_proxy_closes_once_its_peripherals_are_gone_a_while() {
        let start = Instant::now();
        let mut held = Held::default();
        assert!(
            !held.done(start + LINK_GRACE * 10),
            "nothing connected yet: still scanning"
        );
        let first = held.link(Link::default());
        assert!(!held.done(start + LINK_GRACE * 10), "connected");
        held.unlink(first, start);
        assert!(
            !held.done(start + LINK_GRACE - Duration::from_millis(1)),
            "within the grace"
        );
        assert!(held.done(start + LINK_GRACE), "gone a while");
        let second = held.link(Link::default());
        assert_ne!(first, second, "a new handle");
        assert!(!held.done(start + LINK_GRACE * 2), "connected again");
        assert!(held.unlink(first, start).is_none(), "already gone");
    }

    #[test]
    fn base64_round_trips() {
        for bytes in [
            &[][..],
            &[1],
            &[1, 2],
            &[0x65, 0x6c, 0x04, 0, 0, 0, 0xF7, 0, 6],
        ] {
            assert_eq!(
                decode_base64(&encode_base64(bytes)).as_deref(),
                Some(bytes),
                "{bytes:?}"
            );
        }
        assert_eq!(decode_base64("not base64!"), None, "rejected");
    }
}
