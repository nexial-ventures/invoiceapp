//! Design editor calls (design.md, 4f): raw draft uploads and the routes
//! as a given token.

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use serde_json::Value;

use super::auth::exchange;
use super::{TEST_HOST, TEST_TOKEN};

/// A template the mock renders fine, tagged so a render shows which one it was.
pub fn template(tag: &str) -> Vec<u8> {
    format!("// {tag}\n#let data = json(\"/data.json\")\n").into_bytes()
}

pub async fn upload_as(
    app: &Router,
    token: &str,
    path: &str,
    bytes: Vec<u8>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::PUT)
        .uri(format!("/api/pdf/design/draft/file?path={path}"))
        .header("host", TEST_HOST)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(bytes))
        .expect("build request");
    let (status, _, json) = exchange(app, req).await;
    (status, json)
}

/// `PUT /api/pdf/design/draft/file` with the owner token.
pub async fn upload(app: &Router, path: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    upload_as(app, TEST_TOKEN, path, bytes).await
}

pub async fn upload_ok(app: &Router, path: &str, bytes: Vec<u8>) -> Value {
    let (status, listing) = upload(app, path, bytes).await;
    assert_eq!(status, StatusCode::OK, "{path}: {listing}");
    listing
}

pub async fn listing(app: &Router) -> Value {
    let (status, listing) = super::call(app, Method::GET, "/api/pdf/design", None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    listing
}

pub async fn activate(app: &Router) -> (StatusCode, Value) {
    super::call(app, Method::POST, "/api/pdf/design/activate", None).await
}

pub async fn activate_ok(app: &Router) -> Value {
    let (status, body) = activate(app).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["version"].clone()
}

/// Paths of a listing's `files` with the given `source`.
pub fn paths(listing: &Value, source: &str) -> Vec<String> {
    listing["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter(|f| f["source"] == source)
        .filter_map(|f| f["path"].as_str().map(str::to_string))
        .collect()
}
