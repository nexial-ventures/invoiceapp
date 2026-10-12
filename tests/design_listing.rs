//! The live design listing is reused between renders and dropped by the
//! editor's activation (design.md); embedded files go out by their
//! precomputed digest.

mod common;

use axum::http::StatusCode;
use bytes::Bytes;
use common::TestDb;
use common::design::{activate_ok, template, upload_ok};
use common::mdcast::{PdfEnv, Rendered, get_raw};

async fn preview(app: &axum::Router, env: &PdfEnv) -> Rendered {
    let (status, _, _) = get_raw(app, "/api/pdf/preview").await;
    assert_eq!(status, StatusCode::OK);
    env.mock.last()
}

#[tokio::test]
async fn activation_drops_the_cached_listing() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let first = preview(&app, &env).await;
    assert_eq!(first.data["assets"]["signature"], serde_json::Value::Null);
    let font = first
        .digests
        .iter()
        .find(|(k, _)| k.starts_with("fonts/Inter-"))
        .map(|(_, d)| d.clone())
        .expect("an embedded font");

    // Written behind the editor's back (as `invoice design push` does):
    // the cached listing does not show it yet.
    env.storage
        .storage
        .put(&db.key("design/signature.png"), Bytes::from_static(b"sig"))
        .await
        .expect("live file");
    let second = preview(&app, &env).await;
    assert_eq!(second.data["assets"]["signature"], serde_json::Value::Null);
    assert!(second.digests.values().any(|d| *d == font), "same digest");

    upload_ok(&app, "invoice.typ", template("v1")).await;
    activate_ok(&app).await;
    let third = preview(&app, &env).await;
    assert!(third.template.starts_with("// v1"), "{}", third.template);
    assert_eq!(third.data["assets"]["signature"], "signature.png");
}
