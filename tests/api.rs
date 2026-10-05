use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

const QR: &str = "MT:Y.K9042C00KA0648G00";

fn app() -> Router {
    home_link::app(home_link::open_db(":memory:").unwrap())
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn create_room(app: &Router, name: &str) -> i64 {
    let (status, room) = call(app, "POST", "/api/rooms", Some(json!({ "name": name }))).await;
    assert_eq!(status, StatusCode::CREATED, "{room}");
    room["id"].as_i64().unwrap()
}

#[tokio::test]
async fn health_endpoints_respond() {
    let app = app();
    let (status, body) = call(&app, "GET", "/api/health", None).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "status": "ok" })));
    let (status, _) = call(&app, "GET", "/api/missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rooms_support_create_list_rename_and_delete() {
    let app = app();
    let living = create_room(&app, " リビング ").await;
    let bedroom = create_room(&app, "寝室").await;

    let (status, rooms) = call(&app, "GET", "/api/rooms", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        rooms,
        json!([
            { "id": living, "name": "リビング", "device_count": 0 },
            { "id": bedroom, "name": "寝室", "device_count": 0 },
        ])
    );

    let uri = format!("/api/rooms/{living}");
    let (status, room) = call(&app, "PATCH", &uri, Some(json!({ "name": "居間" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        room,
        json!({ "id": living, "name": "居間", "device_count": 0 })
    );

    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, rooms) = call(&app, "GET", "/api/rooms", None).await;
    assert_eq!(rooms.as_array().unwrap().len(), 1);

    let (status, error) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"], "room_not_found");
    let (status, _) = call(&app, "PATCH", &uri, Some(json!({ "name": "x" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn room_names_must_be_present_and_unique() {
    let app = app();
    let bedroom = create_room(&app, "寝室").await;
    for name in ["", "   ", &"あ".repeat(101)] {
        let (status, error) = call(&app, "POST", "/api/rooms", Some(json!({ "name": name }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?}");
        assert_eq!(error["error"], "invalid_room_name");
    }
    let (status, error) = call(&app, "POST", "/api/rooms", Some(json!({ "name": "寝室" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"], "duplicate_room_name");

    let other = create_room(&app, "書斎").await;
    let uri = format!("/api/rooms/{other}");
    let (status, _) = call(&app, "PATCH", &uri, Some(json!({ "name": "寝室" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let uri = format!("/api/rooms/{bedroom}");
    let (status, _) = call(&app, "PATCH", &uri, Some(json!({ "name": "寝室" }))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn rooms_with_devices_cannot_be_deleted() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let device = json!({ "room_id": room, "qr_payload": QR, "name": "" });
    let (status, created) = call(&app, "POST", "/api/devices", Some(device)).await;
    assert_eq!(status, StatusCode::CREATED);

    let uri = format!("/api/rooms/{room}");
    let (status, error) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"], "room_has_devices");
    assert!(error["message"].as_str().unwrap().contains('1'));

    let (_, rooms) = call(&app, "GET", "/api/rooms", None).await;
    assert_eq!(rooms[0]["device_count"], 1);

    let device_uri = format!("/api/devices/{}", created["id"]);
    let (status, _) = call(&app, "DELETE", &device_uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn devices_register_with_optional_name_and_hide_payload_in_list() {
    let app = app();
    let room = create_room(&app, "寝室").await;

    let (status, unnamed) = call(
        &app,
        "POST",
        "/api/devices",
        Some(json!({ "room_id": room, "qr_payload": QR })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{unnamed}");
    assert_eq!(unnamed["name"], "");
    assert_eq!(unnamed["room_id"], room);
    assert_eq!(unnamed["room_name"], "寝室");
    assert!(unnamed.get("qr_payload").is_none());

    let second = "MT:Y.K9042C00KA0648G00*Y.K9042C00KA0648G00";
    let (status, named) = call(
        &app,
        "POST",
        "/api/devices",
        Some(json!({ "room_id": room, "qr_payload": second, "name": " 天井灯 " })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(named["name"], "天井灯");

    let (status, devices) = call(&app, "GET", "/api/devices", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(devices.as_array().unwrap().len(), 2);
    assert!(!devices.to_string().contains("Y.K9042C00"));

    let uri = format!("/api/devices/{}", named["id"]);
    let (status, detail) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["qr_payload"], second);
    assert_eq!(detail["name"], "天井灯");

    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, error) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"], "device_not_found");
}

#[tokio::test]
async fn device_registration_rejects_invalid_input_and_duplicates() {
    let app = app();
    let room = create_room(&app, "寝室").await;
    let post = |body: Value| {
        let app = app.clone();
        async move { call(&app, "POST", "/api/devices", Some(body)).await }
    };

    let (status, error) = post(json!({ "room_id": room, "qr_payload": "MT:abc" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_qr_payload");
    assert!(!error.to_string().contains("MT:abc"));

    let (status, error) = post(json!({ "room_id": room + 99, "qr_payload": QR })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"], "room_not_found");

    let long_name = "あ".repeat(101);
    let (status, error) =
        post(json!({ "room_id": room, "qr_payload": QR, "name": long_name })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_device_name");

    let (status, _) = post(json!({ "room_id": room, "qr_payload": QR })).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, error) = post(json!({ "room_id": room, "qr_payload": format!(" {QR}\n") })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"], "duplicate_qr_payload");

    let (status, _) = post(json!({ "qr_payload": QR })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
