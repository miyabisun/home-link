use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use crate::matter::{Dropped, Socket, connect, receive, request_until, send};

/// A device matterjs-server commissioned, and its own identifiers.
#[derive(Debug, PartialEq, Eq)]
pub struct Commissioned {
    pub node_id: u64,
    /// Basic Information `VendorName` and `SerialNumber`.
    pub identity: Option<(String, String)>,
    /// The Wi-Fi interface's MAC address as 12 upper-case hex digits.
    pub mac: Option<String>,
}

#[derive(Debug)]
pub enum Error {
    /// The WebSocket API could not be reached or dropped the connection.
    Unreachable(String),
    Failed(Failure),
}

/// Paths read after commissioning when the node's cache lacks the identity.
const IDENTITY_PATHS: [&str; 3] = ["0/40/1", "0/40/15", "0/51/0"];
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The network a device joins once commissioned.
pub enum Network<'a> {
    /// Wi-Fi, whose credentials are handed to matterjs-server first.
    Wifi { ssid: &'a str, password: &'a str },
    /// Thread, with the dataset matterjs-server already holds.
    Thread,
    /// Whichever the device reports to matterjs-server: Wi-Fi credentials,
    /// when there are some, are handed over first, and Thread uses the dataset.
    Any(Option<(&'a str, &'a str)>),
}

/// Hands matterjs-server the Wi-Fi credentials (for Wi-Fi), then has it
/// commission the device behind `code` over Bluetooth (not `network_only`),
/// and reads the new node's identifiers. The password goes only to matterjs-server.
///
/// # Errors
/// Fails when matterjs-server cannot be reached, has no Bluetooth or, for
/// Thread, no dataset, refuses the credentials or the device, or does not
/// finish within `timeout`.
pub async fn commission(
    url: &str,
    code: &str,
    network: &Network<'_>,
    timeout: Duration,
) -> Result<Commissioned, Error> {
    let deadline = Instant::now() + timeout;
    let mut ws = connect(url).await.map_err(Error::Unreachable)?;
    let result = session(&mut ws, code, network, deadline).await;
    // Refused before anything was sent, there is nothing to forget.
    if !matches!(
        result,
        Err(Error::Failed(
            Failure::BluetoothDisabled | Failure::ThreadNotReady
        ))
    ) {
        forget_wifi(&mut ws).await;
    }
    let _ = ws.close(None).await;
    result
}

const FORGET_TIMEOUT: Duration = Duration::from_secs(5);

/// Clears the Wi-Fi credentials matterjs-server keeps once handed them; nothing needs them after.
async fn forget_wifi(ws: &mut Socket) {
    let request = json!({ "message_id": "forget", "command": "remove_wifi_credentials" });
    match call(ws, &request, Instant::now() + FORGET_TIMEOUT).await {
        Ok(reply) if error(&reply).is_none() => {}
        _ => tracing::warn!("matterjs-server may still hold Wi-Fi credentials"),
    }
}

/// The Wi-Fi network a device is on, from Network Commissioning (0x0031) on the root endpoint.
#[derive(Debug, PartialEq, Eq)]
pub struct WifiNetwork {
    /// `NetworkID`, base64 as matterjs-server sends bytes.
    pub id: String,
    pub ssid: String,
}

/// The connected network of a device with the Wi-Fi feature; `None` on Thread or Ethernet.
#[must_use]
pub fn wifi_network(attributes: &Value) -> Option<WifiNetwork> {
    if attributes["0/49/65532"].as_u64()? & 1 == 0 {
        return None;
    }
    let id = attributes["0/49/1"]
        .as_array()?
        .iter()
        .find(|network| network["1"] == true)?["0"]
        .as_str()?;
    Some(WifiNetwork {
        id: id.to_owned(),
        ssid: String::from_utf8_lossy(&base64(id)?).into_owned(),
    })
}

/// How long a Wi-Fi move may take at each step.
#[derive(Clone, Copy, Debug)]
pub struct MoveTimes {
    /// The fail-safe armed on the device: unless completed in time, it goes back to its network.
    pub fail_safe: Duration,
    /// The wait for the answer to `ConnectNetwork`, which a device leaving the network rarely sends.
    pub connect: Duration,
    /// The pause between tries to complete on the new network or read the device back.
    pub retry: Duration,
    /// How long the device has to come back after the fail-safe.
    pub settle: Duration,
}

pub const MOVE_TIMES: MoveTimes = MoveTimes {
    fail_safe: Duration::from_mins(2),
    connect: Duration::from_secs(30),
    retry: Duration::from_secs(3),
    settle: Duration::from_mins(1),
};

/// What became of a Wi-Fi move.
#[derive(Debug, PartialEq, Eq)]
pub enum Move {
    /// The device is on the new network for good.
    Done,
    /// The device did not stay on the new network and answers on its old one.
    Reverted,
    /// The device did not stay on the new network and has not answered since.
    Reverting,
}

/// Moves node `node_id` from network `from` (its `NetworkID`) onto Wi-Fi `ssid`
/// without resetting it: under a fail-safe it swaps the one network it holds,
/// joins, and completes on the new network, else the fail-safe takes it back.
/// The password goes only to the device, through matterjs-server.
///
/// # Errors
/// Fails when matterjs-server cannot be reached or drops the connection.
pub async fn move_wifi(
    url: &str,
    node_id: u64,
    from: &str,
    ssid: &str,
    password: &str,
    times: MoveTimes,
) -> Result<Move, String> {
    let mut ws = connect(url).await?;
    let result = moving(&mut ws, node_id, from, ssid, password, times).await;
    forget_wifi(&mut ws).await;
    let _ = ws.close(None).await;
    result.map_err(|Dropped(error)| error)
}

async fn moving(
    ws: &mut Socket,
    node_id: u64,
    from: &str,
    ssid: &str,
    password: &str,
    times: MoveTimes,
) -> Result<Move, Dropped> {
    // matterjs-server greets every connection with its server info.
    receive(ws).await?;
    let command = |cluster: u16, name: &str, payload: Value| {
        json!({ "command": "device_command", "args": {
            "node_id": node_id, "endpoint_id": 0, "cluster_id": cluster,
            "command_name": name, "payload": payload,
        }})
    };
    let arm = |seconds: u64| {
        command(
            GENERAL_COMMISSIONING,
            "ArmFailSafe",
            json!({ "expiryLengthSeconds": seconds, "breadcrumb": 0 }),
        )
    };
    let new = encode_base64(ssid.as_bytes());
    let expires = Instant::now() + times.fail_safe;
    let steps = [
        (arm(times.fail_safe.as_secs()), "errorCode"),
        (
            command(
                NETWORK_COMMISSIONING,
                "RemoveNetwork",
                json!({ "networkId": from, "breadcrumb": 0 }),
            ),
            "networkingStatus",
        ),
        (
            command(
                NETWORK_COMMISSIONING,
                "AddOrUpdateWiFiNetwork",
                json!({ "ssid": new, "credentials": encode_base64(password.as_bytes()), "breadcrumb": 0 }),
            ),
            "networkingStatus",
        ),
    ];
    let mut prepared = true;
    for (step, status) in steps {
        if !succeeded(request_until(ws, step, expires).await?.as_ref(), status) {
            prepared = false;
            break;
        }
    }
    let connect = command(
        NETWORK_COMMISSIONING,
        "ConnectNetwork",
        json!({ "networkId": new, "breadcrumb": 0 }),
    );
    let joined = prepared
        // A device leaving its network rarely answers; one refusing the new network does.
        && match request_until(ws, connect, expires.min(Instant::now() + times.connect)).await? {
            Some(reply) if reply.get("error_code").is_none() => succeeded(Some(&reply), "networkingStatus"),
            _ => true,
        };
    if joined {
        let complete = || command(GENERAL_COMMISSIONING, "CommissioningComplete", json!({}));
        while Instant::now() < expires {
            if succeeded(
                request_until(ws, complete(), expires).await?.as_ref(),
                "errorCode",
            ) {
                return Ok(Move::Done);
            }
            tokio::time::sleep_until(expires.min(Instant::now() + times.retry)).await;
        }
    }
    // Ending the fail-safe takes the device back at once; else it goes back when the fail-safe runs out.
    let disarm = request_until(ws, arm(0), Instant::now() + times.connect).await?;
    if !succeeded(disarm.as_ref(), "errorCode") {
        tokio::time::sleep_until(expires).await;
    }
    let settled = Instant::now() + times.settle;
    let read = || json!({ "command": "read_attribute", "args": { "node_id": node_id, "attribute_path": ["0/49/1"] } });
    while Instant::now() < settled {
        if let Some(reply) = request_until(ws, read(), settled).await?
            && let Some(networks) = reply["result"]["0/49/1"].as_array()
        {
            let on_new = networks
                .iter()
                .any(|n| n["1"] == true && n["0"] == new.as_str());
            return Ok(if on_new { Move::Done } else { Move::Reverted });
        }
        tokio::time::sleep_until(settled.min(Instant::now() + times.retry)).await;
    }
    Ok(Move::Reverting)
}

const GENERAL_COMMISSIONING: u16 = 0x30;
const NETWORK_COMMISSIONING: u16 = 0x31;

/// A reply carrying `status` 0, the success of both commissioning clusters' responses.
fn succeeded(reply: Option<&Value>, status: &str) -> bool {
    reply.is_some_and(|reply| reply.get("error_code").is_none() && reply["result"][status] == 0)
}

/// Whether matterjs-server holds a Thread dataset to commission Thread devices with.
///
/// # Errors
/// Fails when matterjs-server cannot be reached in time.
pub async fn thread_ready(url: &str, timeout: Duration) -> Result<bool, String> {
    tokio::time::timeout(timeout, async {
        let mut ws = connect(url).await?;
        // matterjs-server greets every connection with its server info.
        let info = receive(&mut ws).await.map_err(|e| e.0);
        let _ = ws.close(None).await;
        Ok(info?["thread_credentials_set"] == true)
    })
    .await
    .map_err(|_| "timed out".to_owned())?
}

async fn session(
    ws: &mut Socket,
    code: &str,
    network: &Network<'_>,
    deadline: Instant,
) -> Result<Commissioned, Error> {
    // matterjs-server greets every connection with its server info.
    let info = within(deadline, receive(ws)).await?;
    if info["bluetooth_enabled"] == false {
        return Err(Error::Failed(Failure::BluetoothDisabled));
    }
    let wifi = match network {
        Network::Thread if info["thread_credentials_set"] != true => {
            return Err(Error::Failed(Failure::ThreadNotReady));
        }
        Network::Thread => None,
        Network::Wifi { ssid, password } => Some((*ssid, *password)),
        Network::Any(wifi) => *wifi,
    };
    if let Some((ssid, password)) = wifi {
        let wifi = json!({
            "message_id": "wifi",
            "command": "set_wifi_credentials",
            "args": { "ssid": ssid, "credentials": password },
        });
        let reply = call(ws, &wifi, deadline).await?;
        if let Some(details) = error(&reply) {
            return Err(Error::Failed(Failure::Other(details)));
        }
    }
    let request = json!({
        "message_id": "commission",
        "command": "commission_with_code",
        "args": { "code": code, "network_only": false },
    });
    let mut reply = call(ws, &request, deadline).await?;
    if let Some(details) = error(&reply) {
        return Err(Error::Failed(classify(&details)));
    }
    let node = reply["result"].take();
    let node_id = node["node_id"]
        .as_u64()
        .ok_or_else(|| Error::Unreachable("commission_with_code returned no node_id".into()))?;
    let mut attributes = node["attributes"].clone();
    let wifi = matches!(network, Network::Wifi { .. });
    if identity(&attributes).is_none() || (wifi && wifi_mac(&attributes).is_none()) {
        // A freshly commissioned node may not be in matterjs-server's cache yet.
        let read = json!({
            "message_id": "identity",
            "command": "read_attribute",
            "args": { "node_id": node_id, "attribute_path": IDENTITY_PATHS },
        });
        let deadline = deadline.min(Instant::now() + READ_TIMEOUT);
        if let Ok(Value::Object(read)) = call(ws, &read, deadline)
            .await
            .map(|mut reply| reply["result"].take())
        {
            if !attributes.is_object() {
                attributes = json!({});
            }
            for (path, value) in read {
                attributes[&path] = value;
            }
        }
    }
    Ok(Commissioned {
        node_id,
        identity: identity(&attributes),
        mac: wifi_mac(&attributes),
    })
}

/// Sends `request` and waits for the reply under its message id, skipping events.
async fn call(ws: &mut Socket, request: &Value, deadline: Instant) -> Result<Value, Error> {
    within(deadline, send(ws, request)).await?;
    loop {
        let reply = within(deadline, receive(ws)).await?;
        if reply["message_id"] == request["message_id"] {
            return Ok(reply);
        }
    }
}

async fn within<T>(
    deadline: Instant,
    future: impl Future<Output = Result<T, Dropped>>,
) -> Result<T, Error> {
    match tokio::time::timeout_at(deadline, future).await {
        Err(_) => Err(Error::Failed(Failure::Timeout)),
        Ok(Err(Dropped(error))) => Err(Error::Unreachable(error)),
        Ok(Ok(value)) => Ok(value),
    }
}

fn error(reply: &Value) -> Option<String> {
    reply.get("error_code").map(|_| {
        reply["details"]
            .as_str()
            .unwrap_or("unknown error")
            .to_owned()
    })
}

/// Why matterjs-server could not commission a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No commissionable device answered over Bluetooth.
    NotFound,
    /// The device was found but refused the setup passcode.
    WrongCode,
    /// The device could not join the Wi-Fi network.
    Wifi,
    /// The device could not join the Thread network.
    Thread,
    /// matterjs-server holds no Thread dataset yet.
    ThreadNotReady,
    /// Commissioning did not finish in time.
    Timeout,
    /// matterjs-server has no Bluetooth to commission with.
    BluetoothDisabled,
    /// Any other error, with matterjs-server's details.
    Other(String),
}

/// Classifies matterjs-server's error details, which carry matter.js's message
/// behind "Commission failed:" with error code 1 whatever went wrong.
#[must_use]
pub fn classify(details: &str) -> Failure {
    let has = |word: &str| details.contains(word);
    if has("WiFi network") || has("Wi-Fi network") {
        Failure::Wifi
    } else if has("Thread network") {
        Failure::Thread
    } else if has("timed out") || has("maximum timeframe") || has("Timeout") {
        Failure::Timeout
    } else if has("No commissionable device") || has("Node not found") || has("No device found") {
        Failure::NotFound
    } else if [
        "[ble]",
        "BLE proxy",
        "Unknown command",
        "peripheral disconnected",
        "[peer-communication] Could not connect to device",
    ]
    .iter()
    .any(|word| has(word))
    {
        // The Bluetooth link failed, not the passcode; show matter.js's causes.
        Failure::Other(details.to_owned())
    } else if has("PASE") || has("attempt(s) failed") {
        // Found over Bluetooth, but the passcode exchange failed.
        Failure::WrongCode
    } else {
        Failure::Other(details.to_owned())
    }
}

/// Basic Information (0x0028) `VendorName` and `SerialNumber` on the root endpoint.
#[must_use]
pub fn identity(attributes: &Value) -> Option<(String, String)> {
    let text = |path: &str| {
        attributes[path]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    text("0/40/1").zip(text("0/40/15"))
}

/// The hardware address of the Wi-Fi interface (type 1) in General Diagnostics
/// (0x0033) `NetworkInterfaces`, which matterjs-server sends as base64.
#[must_use]
pub fn wifi_mac(attributes: &Value) -> Option<String> {
    attributes["0/51/0"]
        .as_array()?
        .iter()
        .find_map(|interface| {
            if interface["7"] != 1 {
                return None;
            }
            let bytes = base64(interface["4"].as_str()?)?;
            (bytes.len() == 6).then(|| {
                bytes
                    .iter()
                    .fold(String::new(), |hex, b| hex + &format!("{b:02X}"))
            })
        })
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
fn base64(text: &str) -> Option<Vec<u8>> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        Failure, WifiNetwork, base64, classify, encode_base64, identity, wifi_mac, wifi_network,
    };

    #[test]
    fn base64_round_trips_every_length() {
        for text in ["", "a", "ab", "abc", "iot-2g", "home-2g"] {
            let encoded = encode_base64(text.as_bytes());
            assert_eq!(
                base64(&encoded).as_deref(),
                Some(text.as_bytes()),
                "{encoded}"
            );
        }
        assert_eq!(encode_base64(b"iot-2g"), "aW90LTJn");
        assert_eq!(encode_base64(b"home-2g"), "aG9tZS0yZw==");
    }

    #[test]
    fn the_wifi_network_is_the_connected_one_on_a_wifi_device() {
        let attributes = |features: u64| {
            json!({ "0/49/65532": features, "0/49/1": [
                { "0": "aG9tZS0yZw==", "1": false },
                { "0": "aW90LTJn", "1": true },
            ]})
        };
        let iot = WifiNetwork {
            id: "aW90LTJn".into(),
            ssid: "iot-2g".into(),
        };
        assert_eq!(wifi_network(&attributes(1)), Some(iot));
        // Thread, Ethernet, or nothing connected.
        assert_eq!(wifi_network(&attributes(2)), None);
        assert_eq!(wifi_network(&attributes(4)), None);
        assert_eq!(
            wifi_network(&json!({ "0/49/65532": 1, "0/49/1": [{ "0": "aW90LTJn", "1": false }] })),
            None
        );
        assert_eq!(wifi_network(&json!({})), None);
    }

    #[test]
    fn failures_are_classified_from_matter_js_messages() {
        // Messages as matter.js words them, behind matterjs-server's prefix.
        let cases = [
            (
                "Commission failed: commissioning discovery failed: No commissionable device was discovered",
                Failure::NotFound,
            ),
            (
                "Commission failed: commissioning discovery failed: No device could be commissioned (1 of 1 started attempt(s) failed, 1 discovered)",
                Failure::WrongCode,
            ),
            (
                "Commission failed: Establishing PASE channel failed with channel status response error Received general error status for protocol 0",
                Failure::WrongCode,
            ),
            (
                "Commission failed: Commissionee failed to connect to WiFi network \"home\": AuthFail",
                Failure::Wifi,
            ),
            (
                "Commission failed: Commissionee failed to add WiFi network \"home\"",
                Failure::Wifi,
            ),
            (
                "Commission failed: Commissionee failed to connect to Thread network \"home\": NetworkNotFound",
                Failure::Thread,
            ),
            (
                "Commission failed: Commissioning time exceeds the maximum timeframe of 300s",
                Failure::Timeout,
            ),
            (
                "Commission failed: commissioning discovery failed: No device could be commissioned (1 attempt(s) started of 1 discovered, all canceled or timed out)",
                Failure::Timeout,
            ),
            (
                "Commission failed: Commission error: This device is already commissioned into this fabric.",
                Failure::Other(
                    "Commission failed: Commission error: This device is already commissioned into this fabric."
                        .into(),
                ),
            ),
        ];
        for (details, failure) in cases {
            assert_eq!(classify(details), failure, "{details}");
        }
    }

    #[test]
    fn bluetooth_proxy_failures_are_not_a_wrong_code() {
        // Observed 2026-10-20 from matterjs-server 1.4.0 commissioning over a BLE proxy;
        // `None` keeps matter.js's details as a commission_failed.
        let failed = "Commission failed: commissioning discovery failed: No device could be commissioned (1 of 1 started attempt(s) failed, 1 discovered)";
        let lost =
            "Cause #0: [ble] No connected BLE proxy client owns peripheral 00:00:5E:00:53:01";
        let cases = [
            // 08:09
            (
                "Commission failed: commissioning discovery failed: No commissionable device was discovered".to_owned(),
                Some(Failure::NotFound),
            ),
            // 08:13
            (
                format!("{failed}\nCause #0: internal_error: Unknown command: write_and_subscribe"),
                None,
            ),
            // 08:16
            (format!("{failed}\n{lost}"), None),
            // 08:20
            (
                "Commission failed: commissioning discovery failed: No device could be commissioned (1 attempt(s) started of 1 discovered, all canceled or timed out)".to_owned(),
                Some(Failure::Timeout),
            ),
            // 08:23
            (format!("{failed}\n{lost}"), None),
            // 11:35
            (
                format!("{failed}\n{lost}\nCause #1: internal_error: peripheral disconnected (state=disconnected)"),
                None,
            ),
            // 11:38
            (
                format!("{failed}\nCause #0: [peer-communication] Could not connect to device"),
                None,
            ),
            // 11:50
            (
                format!("{failed}\nCause #0: internal_error: peripheral disconnected (state=disconnected)"),
                None,
            ),
        ];
        for (details, failure) in cases {
            let failure = failure.unwrap_or_else(|| Failure::Other(details.clone()));
            assert_eq!(classify(&details), failure, "{details}");
        }
    }

    #[test]
    fn the_wifi_mac_is_the_hardware_address_of_the_wifi_interface() {
        // General Diagnostics NetworkInterfaces: Thread first, then Wi-Fi (type 1).
        let attributes = json!({ "0/51/0": [
            { "0": "thread0", "4": "AAECAwQFBgc=", "7": 4 },
            { "0": "wlan0", "4": "qrvM3e7/", "7": 1 },
        ]});
        assert_eq!(wifi_mac(&attributes), Some("AABBCCDDEEFF".into()));
        assert_eq!(
            wifi_mac(&json!({ "0/51/0": [{ "4": "qrvM3e7/", "7": 2 }] })),
            None
        );
        assert_eq!(
            wifi_mac(&json!({ "0/51/0": [{ "4": "not base64", "7": 1 }] })),
            None
        );
        assert_eq!(wifi_mac(&json!({})), None);
    }

    #[test]
    fn identity_is_the_basic_information_vendor_and_serial() {
        let attributes = json!({ "0/40/1": "Tapo", "0/40/15": "AABBCCDDEEFF" });
        assert_eq!(
            identity(&attributes),
            Some(("Tapo".into(), "AABBCCDDEEFF".into()))
        );
        assert_eq!(identity(&json!({ "0/40/1": "Tapo", "0/40/15": " " })), None);
        assert_eq!(identity(&json!({ "0/40/15": "S" })), None);
    }
}
