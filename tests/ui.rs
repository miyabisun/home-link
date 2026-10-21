use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

fn app() -> Router {
    home_link::app(home_link::open_db(":memory:").unwrap(), None)
}

async fn get(method: &str, uri: &str) -> (StatusCode, String, String) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let response = app().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        content_type,
        String::from_utf8_lossy(&body).into_owned(),
    )
}

#[tokio::test]
async fn the_admin_screen_is_served_at_the_root_and_its_routes() {
    for uri in ["/", "/labels", "/devices/3", "/labels/7"] {
        let (status, content_type, body) = get("GET", uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(content_type, "text/html", "{uri}");
        assert!(body.starts_with("<!doctype html>"), "{uri}");
    }
}

#[tokio::test]
async fn compiled_assets_are_served_with_their_content_types() {
    let (_, _, html) = get("GET", "/").await;
    for (attribute, extension, content_type) in [
        ("src=\"", ".js", "text/javascript"),
        ("href=\"", ".css", "text/css"),
    ] {
        let asset = html
            .split(attribute)
            .skip(1)
            .filter_map(|part| part.split('"').next())
            .find(|path| path.ends_with(extension))
            .expect("compiled HTML references its asset");
        let (status, served_type, body) = get("GET", asset).await;
        assert_eq!(
            (status, served_type.as_str()),
            (StatusCode::OK, content_type)
        );
        assert!(!body.is_empty() && !body.starts_with("<!doctype html>"));
    }
}

#[tokio::test]
async fn the_api_and_mcp_do_not_fall_back_to_the_screen() {
    for uri in ["/api", "/api/", "/api/missing"] {
        let (status, content_type, _) = get("GET", uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(content_type, "application/json", "{uri}");
    }
    let (status, _, _) = get("GET", "/mcp").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let (status, _, _) = get("POST", "/labels").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}
