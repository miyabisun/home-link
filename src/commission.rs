use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use crate::matter::{Dropped, Socket, connect, receive, send};

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
    let _ = ws.close(None).await;
    result
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
    match network {
        Network::Thread if info["thread_credentials_set"] != true => {
            return Err(Error::Failed(Failure::ThreadNotReady));
        }
        Network::Thread => {}
        Network::Wifi { ssid, password } => {
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

    use super::{Failure, classify, identity, wifi_mac};

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
    fn the_wifi_mac_is_the_hardware_address_of_the_wifi_interface() {
        // General Diagnostics NetworkInterfaces: Thread first, then Wi-Fi (type 1).
        let attributes = json!({ "0/51/0": [
            { "0": "thread0", "4": "AAECAwQFBgc=", "7": 4 },
            { "0": "wlan0", "4": "zLq94MJE", "7": 1 },
        ]});
        assert_eq!(wifi_mac(&attributes), Some("CCBABDE0C244".into()));
        assert_eq!(
            wifi_mac(&json!({ "0/51/0": [{ "4": "zLq94MJE", "7": 2 }] })),
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
        let attributes = json!({ "0/40/1": "Tapo", "0/40/15": "CCBABDE0C244" });
        assert_eq!(
            identity(&attributes),
            Some(("Tapo".into(), "CCBABDE0C244".into()))
        );
        assert_eq!(identity(&json!({ "0/40/1": "Tapo", "0/40/15": " " })), None);
        assert_eq!(identity(&json!({ "0/40/15": "S" })), None);
    }
}
