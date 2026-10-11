//! Design activation, versions, restore, reset, pruning and the rate limit
//! (design.md, 4f); activation on both storage backends.

mod common;

use axum::http::{Method, StatusCode};
use bytes::Bytes;
use common::design::{activate, activate_ok, listing, paths, template, upload_ok};
use common::mdcast::{PdfEnv, get_raw};
use common::storage::TestStorage;
use common::{TestDb, call};
use serde_json::{Value, json};

/// The template the next live render (the 1d preview) used.
async fn live_template(app: &axum::Router, env: &PdfEnv) -> String {
    let (status, _, _) = get_raw(app, "/api/pdf/preview").await;
    assert_eq!(status, StatusCode::OK);
    env.mock.last().template
}

async fn versions(app: &axum::Router) -> Vec<Value> {
    let (status, v) = call(app, Method::GET, "/api/pdf/design/versions", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v.as_array().expect("versions").clone()
}

#[tokio::test]
async fn activation_fs() {
    activation(TestStorage::fs()).await;
}

#[tokio::test]
async fn activation_s3() {
    activation(TestStorage::s3()).await;
}

async fn activation(storage: TestStorage) {
    let db = TestDb::new().await;
    let env = PdfEnv::with_storage(storage);
    let app = env.router(db.conn.clone());
    env.storage
        .storage
        .put(&db.key("design/logo.png"), Bytes::from_static(b"old png"))
        .await
        .expect("live custom file");
    // Cached by the first live render.
    assert!(!live_template(&app, &env).await.starts_with("// v1"));

    upload_ok(&app, "invoice.typ", template("v1")).await;
    upload_ok(&app, "fonts/brand.ttf", b"ttf".to_vec()).await;
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/pdf/design/draft/file?path=logo.png",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let before = env.mock.count();
    let version = activate_ok(&app).await;
    let renders: Vec<_> = env.mock.renders()[before..].to_vec();
    let locales: Vec<_> = renders.iter().map(|r| r.data["locale"].clone()).collect();
    assert_eq!(
        locales,
        [json!("cs"), json!("en")],
        "checked in both locales"
    );
    assert!(renders.iter().all(|r| r.template.starts_with("// v1")));

    assert_eq!(
        (version["number"].clone(), version["kind"].clone()),
        (json!(1), json!("activation"))
    );
    assert_eq!(version["current"], true);
    assert_eq!(version["createdBy"]["email"], common::OWNER_EMAIL);
    let files: Vec<&str> = version["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert_eq!(files, ["fonts/brand.ttf", "invoice.typ"]);
    assert_eq!(
        version["files"][0]["sha256"],
        invoice::storage::sha256_hex(b"ttf")
    );

    let l = listing(&app).await;
    assert_eq!(l["draft"], Value::Null, "draft cleared");
    assert_eq!(paths(&l, "custom"), ["fonts/brand.ttf", "invoice.typ"]);
    assert!(env.storage.keys(&db.key("design-draft")).await.is_empty());
    let id = version["id"].as_str().expect("id");
    assert_eq!(
        env.storage
            .bytes(&db.key(&format!("design-versions/{id}/invoice.typ")))
            .await,
        Some(template("v1"))
    );
    assert!(live_template(&app, &env).await.starts_with("// v1"));
    assert_eq!(env.mock.last().data["assets"]["logo"], Value::Null);
    assert_eq!(versions(&app).await, [version]);
}

#[tokio::test]
async fn failed_activation_changes_nothing() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let (status, err) = activate(&app).await;
    assert_eq!(
        (status, err),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "code": "draft_empty" })
        )
    );

    upload_ok(&app, "invoice.typ", template("mock-fail-en")).await;
    let (status, err) = activate(&app).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err,
        json!({ "code": "design_invalid", "detail": "en: error: mock template failure" })
    );
    upload_ok(&app, "invoice.typ", template("mock-fail")).await;
    let (_, err) = activate(&app).await;
    assert_eq!(err["detail"], "cs: error: mock template failure");

    let l = listing(&app).await;
    assert!(paths(&l, "custom").is_empty(), "live unchanged");
    assert_eq!(l["draft"]["files"][0]["path"], "invoice.typ", "draft kept");
    assert!(versions(&app).await.is_empty());
    assert!(
        env.storage
            .keys(&db.key("design-versions"))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn restore_and_reset() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    upload_ok(&app, "invoice.typ", template("v1")).await;
    let v1 = activate_ok(&app).await;
    upload_ok(&app, "invoice.typ", template("v2")).await;
    upload_ok(&app, "logo.svg", b"<svg/>".to_vec()).await;
    activate_ok(&app).await;
    assert!(live_template(&app, &env).await.starts_with("// v2"));

    // A pending draft survives a restore.
    upload_ok(&app, "signature.png", b"sig".to_vec()).await;
    let uri = format!(
        "/api/pdf/design/versions/{}/restore",
        v1["id"].as_str().expect("id")
    );
    let (status, body) = call(&app, Method::POST, &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v3 = &body["version"];
    assert_eq!(
        (v3["number"].clone(), v3["kind"].clone()),
        (json!(3), json!("revert"))
    );
    assert_eq!(v3["files"], v1["files"]);
    let l = listing(&app).await;
    assert_eq!(paths(&l, "custom"), ["invoice.typ"], "logo.svg removed");
    assert_eq!(l["draft"]["files"][0]["path"], "signature.png");
    assert!(live_template(&app, &env).await.starts_with("// v1"));

    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/pdf/design/versions/{}/restore", uuid::Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = call(&app, Method::POST, "/api/pdf/design/reset", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"]["kind"], "default");
    assert_eq!(body["version"]["files"], json!([]));
    assert!(paths(&listing(&app).await, "custom").is_empty());
    assert!(!live_template(&app, &env).await.starts_with("// v"));
    let numbers: Vec<_> = versions(&app)
        .await
        .iter()
        .map(|v| v["number"].clone())
        .collect();
    assert_eq!(numbers, [json!(4), json!(3), json!(2), json!(1)]);
    let current: Vec<_> = versions(&app)
        .await
        .iter()
        .map(|v| v["current"].clone())
        .collect();
    assert_eq!(
        current,
        [json!(true), json!(false), json!(false), json!(false)]
    );
}

#[tokio::test]
async fn keeps_the_ten_newest_versions() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    upload_ok(&app, "invoice.typ", template("v1")).await;
    let v1 = activate_ok(&app).await;
    let v1_dir = db.key(&format!(
        "design-versions/{}",
        v1["id"].as_str().expect("id")
    ));
    // A leftover of an interrupted activation is collected too.
    let orphan = db.key(&format!("design-versions/{}/x.png", uuid::Uuid::new_v4()));
    env.storage
        .storage
        .put(&orphan, Bytes::from_static(b"x"))
        .await
        .expect("orphan");
    for _ in 0..9 {
        let (status, _) = call(&app, Method::POST, "/api/pdf/design/reset", None).await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(versions(&app).await.len(), 10);
    assert_eq!(env.storage.keys(&v1_dir).await.len(), 1, "v1 still kept");
    assert!(env.storage.bytes(&orphan).await.is_none());

    let (status, _) = call(&app, Method::POST, "/api/pdf/design/reset", None).await;
    assert_eq!(status, StatusCode::OK);
    let all = versions(&app).await;
    assert_eq!(all.len(), 10);
    assert_eq!(
        (all[0]["number"].clone(), all[9]["number"].clone()),
        (json!(11), json!(2))
    );
    assert!(
        env.storage.keys(&v1_dir).await.is_empty(),
        "objects deleted"
    );
    assert_eq!(listing(&app).await["quota"]["used"], 0);
}

#[tokio::test]
async fn activation_and_restore_are_rate_limited() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    upload_ok(&app, "invoice.typ", template("v0")).await;
    let first = activate_ok(&app).await;
    for i in 1..9 {
        upload_ok(&app, "invoice.typ", template(&format!("v{i}"))).await;
        activate_ok(&app).await;
    }
    let uri = format!(
        "/api/pdf/design/versions/{}/restore",
        first["id"].as_str().expect("id")
    );
    let (status, _) = call(&app, Method::POST, &uri, None).await;
    assert_eq!(status, StatusCode::OK, "the tenth");
    upload_ok(&app, "invoice.typ", template("v10")).await;
    let renders = env.mock.count();
    let (status, err) = activate(&app).await;
    assert_eq!(
        (status, err["code"].clone()),
        (StatusCode::TOO_MANY_REQUESTS, json!("rate_limited"))
    );
    let (status, _) = call(&app, Method::POST, &uri, None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(env.mock.count(), renders, "no render once limited");
    // Reset is not limited.
    let (status, _) = call(&app, Method::POST, "/api/pdf/design/reset", None).await;
    assert_eq!(status, StatusCode::OK);
}
