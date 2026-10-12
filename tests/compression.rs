//! Response compression (issue #40, docs/architecture.md): text bodies are
//! compressed by `Accept-Encoding`, PDFs pass through untouched.

mod common;

use axum::Router;
use axum::http::Method;
use axum::http::{HeaderMap, StatusCode, header};
use common::mdcast::PdfEnv;
use common::{TEST_TOKEN, TestDb, authed, get, router, send};
use serde_json::json;

/// GET `uri` with `Accept-Encoding: encoding`: status, headers, raw body.
async fn fetch(
    app: &Router,
    uri: &str,
    encoding: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut req = get(uri, Some(TEST_TOKEN));
    if let Some(e) = encoding {
        req.headers_mut()
            .insert(header::ACCEPT_ENCODING, e.parse().expect("header"));
    }
    let resp = send(app.clone(), req).await;
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = axum::body::to_bytes(resp.into_body(), 1 << 24)
        .await
        .expect("read body");
    (status, headers, body.to_vec())
}

#[tokio::test]
async fn json_is_compressed_by_accept_encoding() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (status, h, plain) = fetch(&app, "/api/openapi.json", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!h.contains_key(header::CONTENT_ENCODING));
    serde_json::from_slice::<serde_json::Value>(&plain).expect("plain JSON");

    for (accept, expected) in [("gzip", "gzip"), ("br", "br"), ("gzip, br", "br")] {
        let (status, h, body) = fetch(&app, "/api/openapi.json", Some(accept)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(h[header::CONTENT_ENCODING], expected, "{accept}");
        assert_eq!(h[header::CONTENT_TYPE], "application/json");
        assert!(
            h.get_all(header::VARY)
                .iter()
                .any(|v| v.to_str().is_ok_and(|v| v.contains("accept-encoding"))),
            "{h:?}"
        );
        assert!(body.len() < plain.len() / 2, "{accept}: {}", body.len());
        // The security headers still apply to a compressed response.
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    }
}

#[tokio::test]
async fn a_pdf_is_not_compressed() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let (status, h, body) = fetch(&app, "/api/pdf/preview", Some("gzip, br")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "application/pdf");
    assert!(!h.contains_key(header::CONTENT_ENCODING), "{h:?}");
    assert_eq!(body, env.mock.last().pdf);
}

#[tokio::test]
async fn a_fresh_token_is_never_compressed() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = json!({ "name": "ci", "role": "member", "expiresAt": null });
    let mut req = authed(Method::POST, "/api/tokens", Some(body));
    req.headers_mut()
        .insert(header::ACCEPT_ENCODING, "gzip, br".parse().expect("header"));
    let resp = send(app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(resp.headers()[header::CONTENT_ENCODING], "identity");
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    let created: serde_json::Value = serde_json::from_slice(&bytes).expect("plain JSON");
    assert!(created["token"].is_string(), "{created}");
}
