//! The app on each storage backend (archive, design overrides) and with the
//! storage unreachable.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{create_doc, get_doc, id, issuable, issue};
use common::mdcast::{PdfEnv, get_raw};
use common::storage::{TestStorage, dead_s3};
use common::{TestDb, call};
use serde_json::{Value, json};

#[tokio::test]
async fn archive_and_design_fs() {
    archive_and_design(TestStorage::fs()).await;
}

#[tokio::test]
async fn archive_and_design_s3() {
    archive_and_design(TestStorage::s3()).await;
}

async fn archive_and_design(storage: TestStorage) {
    let db = TestDb::new().await;
    let env = PdfEnv::with_storage(storage);
    let kind = env.storage.kind();
    let (app, pdf) = env.router_and_pdf(db.conn.clone());
    // The space's design overlay: `spaces/{id}/design/…`.
    let put = |key: &'static str, bytes: &'static [u8]| {
        let (key, storage) = (db.key(key), env.storage.storage.clone());
        async move { storage.put(&key, bytes::Bytes::from_static(bytes)).await }
    };
    put("design/invoice.typ", b"// custom\n#json(\"/data.json\")")
        .await
        .expect("put template");
    put("design/logo.png", b"\x89PNG fake")
        .await
        .expect("put logo");
    put("design/.hidden/x", b"x").await.expect("put hidden");

    let body = issuable(&app).await;
    let (status, doc) = issue(&app, &id(&create_doc(&app, body).await)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let render = env.mock.last();
    assert!(render.template.starts_with("// custom"), "{kind}");
    assert_eq!(
        render.data["assets"],
        json!({ "logo": "logo.png", "signature": null })
    );
    assert!(render.assets.contains(&"logo.png".to_string()));
    assert!(!render.assets.iter().any(|a| a.contains("hidden")));

    let key = db.key(&common::documents::archive_rel(&db.conn, &doc).await);
    assert_eq!(env.storage.bytes(&key).await, Some(render.pdf.clone()));
    let (status, headers, bytes) = get_raw(&app, &format!("/api/documents/{}/pdf", id(&doc))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, render.pdf);
    assert_eq!(
        headers["content-length"],
        render.pdf.len().to_string().as_str()
    );

    let (status, listing) = call(&app, Method::GET, "/api/pdf/design", None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert_eq!(listing["storage"], kind);
    let files = listing["files"].as_array().expect("files");
    let source = |p: &str| {
        files
            .iter()
            .find(|f| f["path"] == p)
            .map(|f| (f["source"].clone(), f["size"].clone()))
    };
    assert_eq!(source("invoice.typ").map(|s| s.0), Some(json!("custom")));
    assert_eq!(source("logo.png"), Some((json!("custom"), json!(9))));
    assert_eq!(
        source("fonts/Inter-Regular.ttf").map(|s| s.0),
        Some(json!("default"))
    );
    assert!(
        !files
            .iter()
            .any(|f| f["path"].as_str().is_some_and(|p| p.contains("hidden")))
    );

    // A replaced override is picked up by the next render once the cached
    // listing is gone (expired, or dropped as the editor does); a removed
    // one falls back to the default.
    put("design/logo.png", b"\x89PNG v2")
        .await
        .expect("replace logo");
    env.storage
        .storage
        .delete(&db.key("design/invoice.typ"))
        .await
        .expect("remove template");
    pdf.design_cache().invalidate(&db.space.storage_prefix());
    let (status, _, _) = get_raw(&app, "/api/pdf/preview").await;
    assert_eq!(status, StatusCode::OK);
    let render = env.mock.last();
    assert!(render.template.contains("/data.json"));
    assert!(!render.template.starts_with("// custom"));
    assert_eq!(render.data["assets"]["logo"], "logo.png");
}

#[tokio::test]
async fn storage_down_issues_nothing() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let down = env.router_with(db.conn.clone(), &env.url, dead_s3());
    let body = issuable(&down).await;
    let draft = create_doc(&down, body).await;

    let (status, err) = issue(&down, &id(&draft)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{err}");
    assert_eq!(err, json!({ "code": "storage_unavailable" }));
    assert_eq!(env.mock.count(), 0, "the design could not be read");
    let doc = get_doc(&down, &id(&draft)).await;
    assert_eq!(
        (&doc["status"], &doc["number"], &doc["pdf"]),
        (&json!("draft"), &Value::Null, &Value::Null)
    );

    let (status, err) = call(&down, Method::GET, "/api/pdf/design", None).await;
    assert_eq!(
        (status, err),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "code": "storage_unavailable" })
        )
    );

    // The counter did not move; once issued, the archive cannot be served
    // while the storage is down.
    let up = env.router(db.conn.clone());
    let (status, doc) = issue(&up, &id(&draft)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["number"], "20260001");
    let (status, err) = call(
        &down,
        Method::GET,
        &format!("/api/documents/{}/pdf", id(&doc)),
        None,
    )
    .await;
    assert_eq!(
        (status, err),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "code": "storage_unavailable" })
        )
    );
}
