//! A Model Context Protocol server (Streamable HTTP, JSON replies only) for labels.
//! Each tool is a request to the HTTP API beside it, so it checks, behaves and fails
//! as the API does, without authentication like the API.

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use tower::ServiceExt;

/// The protocol version answered when the client asks for one not listed here.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// Answers one JSON-RPC message posted to `/mcp`, calling `api` for the tools.
pub async fn handle(api: Router, body: Bytes) -> Response {
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return reply(
            StatusCode::BAD_REQUEST,
            &Value::Null,
            Err((-32700, "parse error")),
        );
    };
    let Some(id) = message
        .get("id")
        .filter(|_| message.get("method").is_some())
    else {
        // A notification or a response: nothing to answer.
        return StatusCode::ACCEPTED.into_response();
    };
    let params = &message["params"];
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            Ok(json!({
                "protocolVersion": VERSIONS.iter().find(|v| **v == asked).unwrap_or(&VERSIONS[0]),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "home-link", "version": env!("CARGO_PKG_VERSION") },
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call(api, params).await,
        _ => Err((-32601, "method not found")),
    };
    reply(StatusCode::OK, id, result)
}

fn reply(status: StatusCode, id: &Value, result: Result<Value, (i64, &str)>) -> Response {
    let body = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    };
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Calls the API for the tool in `params`; the API's answer is the tool's text, an
/// error status its error.
async fn call(api: Router, params: &Value) -> Result<Value, (i64, &'static str)> {
    let arguments = params["arguments"].as_object().cloned().unwrap_or_default();
    let id = || {
        arguments
            .get("id")
            .and_then(Value::as_i64)
            .ok_or((-32602, "id (an integer) is required"))
    };
    let rest = || {
        let mut rest = arguments.clone();
        rest.remove("id");
        Value::Object(rest)
    };
    let (method, path, body) = match params["name"].as_str().unwrap_or_default() {
        "list_labels" => ("GET", "/api/labels".to_owned(), None),
        "create_label" => ("POST", "/api/labels".to_owned(), Some(rest())),
        "update_label" => ("PATCH", format!("/api/labels/{}", id()?), Some(rest())),
        "delete_label" => ("DELETE", format!("/api/labels/{}", id()?), None),
        "list_devices" => ("GET", "/api/devices".to_owned(), None),
        "set_device_labels" => (
            "PATCH",
            format!("/api/devices/{}", id()?),
            Some(
                json!({ "label_ids": arguments.get("label_ids").cloned().unwrap_or(Value::Null) }),
            ),
        ),
        "label_on" => ("POST", format!("/api/labels/{}/on", id()?), None),
        "label_off" => ("POST", format!("/api/labels/{}/off", id()?), None),
        "label_toggle" => ("POST", format!("/api/labels/{}/toggle", id()?), None),
        _ => return Err((-32602, "unknown tool")),
    };
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    let request = match body {
        Some(body) => request.body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .map_err(|_| (-32603, "internal error"))?;
    let Ok(response) = api.oneshot(request).await;
    let failed = !response.status().is_success();
    let text = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_or_else(
            |_| String::new(),
            |b| String::from_utf8_lossy(&b).into_owned(),
        );
    Ok(json!({
        "content": [{ "type": "text", "text": if text.is_empty() { "null".to_owned() } else { text } }],
        "isError": failed,
    }))
}

/// The tools with the schemas of their arguments.
fn tools() -> Value {
    let id = json!({ "type": "integer", "description": "labelのid（list_labelsで分かる）" });
    let traits = json!({
        "name": { "type": "string", "description": "labelの名前（1〜100文字）" },
        "auto": { "type": "boolean", "description": "照明を明るさと色温度の自動調整に連動させるか（上書きの値とは併用できない）" },
        "override_level": { "type": ["integer", "null"], "minimum": 1, "maximum": 254,
            "description": "ONの間に固定する明るさ（254が100%）。override_kelvinと両方を指定し、nullで外す" },
        "override_kelvin": { "type": ["integer", "null"], "minimum": 1000, "maximum": 10000,
            "description": "ONの間に固定する色温度（K）" },
        "schedule": { "type": "array", "description": "毎日の切り替え（Asia/Tokyo）。送ると置き換え、[]で消す",
            "items": { "type": "object", "required": ["time", "on"], "properties": {
                "time": { "type": "string", "pattern": "^([01][0-9]|2[0-3]):[0-5][0-9]$" },
                "on": { "type": "boolean" } } } },
    });
    let mut update = traits.clone();
    update["id"] = id.clone();
    let by_id = |description: &str| {
        json!({ "description": description,
                "inputSchema": { "type": "object", "required": ["id"], "properties": { "id": id } } })
    };
    let named = |name: &str, mut tool: Value| {
        tool["name"] = json!(name);
        tool
    };
    json!([
        named(
            "list_labels",
            json!({ "description": "labelの一覧（性質・スケジュール・状態・機器の台数）",
            "inputSchema": { "type": "object", "properties": {} } })
        ),
        named(
            "create_label",
            json!({ "description": "labelを作る",
            "inputSchema": { "type": "object", "required": ["name"], "properties": traits } })
        ),
        named(
            "update_label",
            json!({ "description": "labelの名前・性質・スケジュールのうち、送った項目だけを変える",
            "inputSchema": { "type": "object", "required": ["id"], "properties": update } })
        ),
        named(
            "delete_label",
            by_id("labelを消す。機器のほかのlabelは残る")
        ),
        named(
            "list_devices",
            json!({ "description": "台帳の機器の一覧（id・名前・種類・label・接続の状態）",
            "inputSchema": { "type": "object", "properties": {} } })
        ),
        named(
            "set_device_labels",
            json!({ "description": "機器のlabelを置き換える（[]で外す）",
            "inputSchema": { "type": "object", "required": ["id", "label_ids"], "properties": {
                "id": { "type": "integer", "description": "機器のid（list_devicesで分かる）" },
                "label_ids": { "type": "array", "items": { "type": "integer" } } } } })
        ),
        named(
            "label_on",
            by_id("labelの照明とプラグをONにする。上書きの値を持つlabelはその値で点ける")
        ),
        named("label_off", by_id("labelの照明とプラグをOFFにする")),
        named(
            "label_toggle",
            by_id(
                "labelを切り替える。上書き中のlabelは上書きをやめ、照明をほかのlabelの状態へ戻す"
            )
        ),
    ])
}
