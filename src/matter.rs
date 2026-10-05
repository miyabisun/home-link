use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

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

/// Reads every node from the matterjs-server WebSocket API at `url`.
///
/// # Errors
/// Fails when the server cannot be reached, times out, or answers with an error.
pub async fn fetch_nodes(url: &str) -> Result<Vec<Value>, String> {
    tokio::time::timeout(TIMEOUT, request_nodes(url))
        .await
        .map_err(|_| "timed out".to_owned())?
}

async fn request_nodes(url: &str) -> Result<Vec<Value>, String> {
    let (mut ws, _) = connect_async(url).await.map_err(|e| e.to_string())?;
    let request = json!({ "message_id": "get_nodes", "command": "get_nodes" });
    ws.send(Message::text(request.to_string()))
        .await
        .map_err(|e| e.to_string())?;
    while let Some(message) = ws.next().await {
        let Message::Text(text) = message.map_err(|e| e.to_string())? else {
            continue;
        };
        let mut reply: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if reply["message_id"] != "get_nodes" {
            continue;
        }
        let _ = ws.close(None).await;
        return match reply["result"].take() {
            Value::Array(nodes) => Ok(nodes),
            _ => Err(format!("get_nodes failed: {}", reply["error_code"])),
        };
    }
    Err("connection closed before the reply".into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::locate;

    // Trimmed from a matterjs-server `get_nodes` result (2026-10-05).
    fn nodes() -> Vec<serde_json::Value> {
        vec![
            json!({ "node_id": 1, "attributes": {
                "0/40/1": "Aqara", "0/40/15": "54ef4475b119",
                "2/57/1": "Aqara", "2/57/15": "54ef44100126e0a6",
                "3/57/1": "Aqara", "3/57/15": "54ef441001372c4c",
            }}),
            json!({ "node_id": 5, "attributes": { "0/40/1": "Tapo", "0/40/15": "CCBABDE0C244" }}),
            json!({ "node_id": 16, "attributes": {
                "0/40/1": "Uascent", "0/40/15": "U20251209000001453",
            }}),
        ]
    }

    #[test]
    fn locates_root_devices_by_basic_information() {
        assert_eq!(locate(&nodes(), "Tapo", "CCBABDE0C244"), Some((5, 0)));
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
        assert_eq!(locate(&nodes(), "Tapo", "CCBABDE0FFFF"), None);
        assert_eq!(locate(&nodes(), "Aqara", "CCBABDE0C244"), None);
        // A serial number on another cluster's attribute is not an identity.
        let other = vec![json!({ "node_id": 2, "attributes": { "1/6/15": "X", "1/6/1": "V" }})];
        assert_eq!(locate(&other, "V", "X"), None);
        assert_eq!(locate(&[json!({ "node_id": 3 })], "V", "X"), None);
    }
}
