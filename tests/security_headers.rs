//! Security headers on every response (issue #40, docs/api/README.md).

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, header};
use common::{BASE_HOST, TEST_HOST, TestDb, router, send};
use invoiceapp::auth::host::PublicUrl;
use invoiceapp::security_headers::{CSP, HSTS};

async fn headers(app: &Router, host: &str, uri: &str) -> HeaderMap {
    let req = Request::builder()
        .uri(uri)
        .header(header::HOST, host)
        .body(Body::empty())
        .expect("request");
    send(app.clone(), req).await.headers().clone()
}

fn https_app(db: sea_orm::DatabaseConnection) -> Router {
    common::TestApp::builder(&db)
        .state(|s| s.public = PublicUrl::parse("https://localhost:3000").expect("public URL"))
        .build()
}

fn assert_common(h: &HeaderMap, what: &str) {
    assert_eq!(h[header::CONTENT_SECURITY_POLICY], CSP, "{what}");
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{what}");
    assert_eq!(
        h[header::REFERRER_POLICY],
        "strict-origin-when-cross-origin",
        "{what}"
    );
    assert_eq!(h[header::X_FRAME_OPTIONS], "DENY", "{what}");
}

#[tokio::test]
async fn every_response_carries_them_without_hsts_over_http() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    for (host, uri) in [
        (BASE_HOST, "/api/health"),
        (BASE_HOST, "/"),
        (TEST_HOST, "/invoices/42"),
        (TEST_HOST, "/assets/missing-xyz.js"),
        (TEST_HOST, "/api/auth/me"),
        (TEST_HOST, "/api/no-such-route"),
        ("unknown.example.com", "/api/auth/me"),
    ] {
        let h = headers(&app, host, uri).await;
        assert_common(&h, uri);
        assert!(!h.contains_key(header::STRICT_TRANSPORT_SECURITY), "{uri}");
    }
}

#[tokio::test]
async fn hsts_over_https() {
    let db = TestDb::new().await;
    let app = https_app(db.conn.clone());
    for uri in ["/api/health", "/", "/api/auth/me"] {
        let h = headers(&app, BASE_HOST, uri).await;
        assert_common(&h, uri);
        assert_eq!(h[header::STRICT_TRANSPORT_SECURITY], HSTS, "{uri}");
    }
}
