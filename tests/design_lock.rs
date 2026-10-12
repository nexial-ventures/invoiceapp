//! Design editor edge cases (design.md, 4f): replacing existing names,
//! file / directory clashes, early size refusal, the NOWAIT lock
//! (`design_busy`), a draft changed during the activation check
//! (`draft_changed`) and incomplete versions (`version_incomplete`).

mod common;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use bytes::Bytes;
use common::auth::exchange;
use common::design::{activate, activate_ok, listing, template, upload, upload_ok};
use common::mdcast::PdfEnv;
use common::{TEST_HOST, TEST_TOKEN, TestDb, call};
use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use serde_json::{Value, json};

#[tokio::test]
async fn existing_names_can_be_replaced_and_clashes_are_refused() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    env.storage
        .storage
        .put(
            &db.key("design/fonts/Brand.OTF"),
            Bytes::from_static(b"otf"),
        )
        .await
        .expect("CLI-pushed file");
    // Default and live names outside the strict rule may be replaced.
    for path in [
        "fonts/OFL.txt",
        "fonts/Inter-Regular.ttf",
        "fonts/Brand.OTF",
    ] {
        let l = upload_ok(&app, path, b"x".to_vec()).await;
        let file = l["draft"]["files"]
            .as_array()
            .expect("files")
            .iter()
            .find(|f| f["path"] == path)
            .cloned()
            .expect("in the draft");
        assert_eq!(file["change"], "replaced", "{path}");
    }
    upload_ok(&app, "img.png/a.png", b"png".to_vec()).await;
    for path in ["fonts/Other.OTF", "invoice.typ/x.png", "img.png", "qr.svg"] {
        let (status, err) = upload(&app, path, b"x".to_vec()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(err["fields"]["path"], "invalid", "{path}");
    }
}

#[tokio::test]
async fn an_oversized_declared_length_is_refused_before_reading() {
    let db = TestDb::new().await;
    let app = PdfEnv::new().router(db.conn.clone());
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/api/pdf/design/draft/file?path=logo.png")
        .header("host", TEST_HOST)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .header("content-length", (20 * 1024 * 1024).to_string())
        .body(Body::from("tiny"))
        .expect("request");
    let (status, _, err) = exchange(&app, req).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["fields"]["file"], "too_large");
    assert_eq!(listing(&app).await["draft"], Value::Null);
}

#[tokio::test]
async fn a_held_lock_is_design_busy() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    upload_ok(&app, "invoice.typ", template("v1")).await;

    let holder = db.conn.begin().await.expect("begin");
    holder
        .execute(Statement::from_sql_and_values(
            holder.get_database_backend(),
            "SELECT 1 FROM design_drafts WHERE space_id = $1 FOR UPDATE",
            vec![db.space.into()],
        ))
        .await
        .expect("hold the design lock");
    let busy = json!({ "code": "design_busy" });
    assert_eq!(
        upload(&app, "logo.svg", b"<svg/>".to_vec()).await,
        (StatusCode::CONFLICT, busy.clone())
    );
    for (method, uri) in [
        (
            Method::DELETE,
            "/api/pdf/design/draft/file?path=invoice.typ",
        ),
        (Method::DELETE, "/api/pdf/design/draft"),
        (Method::POST, "/api/pdf/design/activate"),
        (Method::POST, "/api/pdf/design/reset"),
    ] {
        let got = call(&app, method.clone(), uri, None).await;
        assert_eq!(got, (StatusCode::CONFLICT, busy.clone()), "{method} {uri}");
    }
    holder.rollback().await.expect("release");

    // Nothing was applied; with the lock free the activation goes through.
    assert!(
        listing(&app).await["files"]
            .as_array()
            .expect("files")
            .iter()
            .all(|f| f["source"] == "default")
    );
    assert_eq!(activate_ok(&app).await["number"], 1);
}

#[tokio::test]
async fn a_draft_changed_during_the_check_is_not_applied() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    upload_ok(&app, "invoice.typ", template("v1")).await;

    env.mock.hold();
    let started = env.mock.started();
    let running = {
        let app = app.clone();
        tokio::spawn(async move { activate(&app).await })
    };
    // The check renders without the lock: an upload while its render is
    // held goes through.
    env.mock.wait_started(started + 1).await;
    upload_ok(&app, "logo.svg", b"<svg/>".to_vec()).await;
    env.mock.release();
    let (status, err) = running.await.expect("activation task");
    assert_eq!(
        (status, err),
        (StatusCode::CONFLICT, json!({ "code": "draft_changed" }))
    );

    let l = listing(&app).await;
    assert!(
        l["files"]
            .as_array()
            .expect("files")
            .iter()
            .all(|f| f["source"] == "default")
    );
    assert_eq!(
        l["draft"]["files"].as_array().map(Vec::len),
        Some(2),
        "draft kept"
    );
    let (_, versions) = call(&app, Method::GET, "/api/pdf/design/versions", None).await;
    assert_eq!(versions, json!([]));
}

#[tokio::test]
async fn an_incomplete_version_is_not_restored() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    upload_ok(&app, "invoice.typ", template("v1")).await;
    upload_ok(&app, "logo.svg", b"<svg/>".to_vec()).await;
    let v1 = activate_ok(&app).await;
    let id = v1["id"].as_str().expect("id");
    upload_ok(&app, "invoice.typ", template("v2")).await;
    activate_ok(&app).await;

    let restore = format!("/api/pdf/design/versions/{id}/restore");
    let logo = db.key(&format!("design-versions/{id}/logo.svg"));
    // A changed size counts as incomplete, a missing file too.
    env.storage
        .storage
        .put(&logo, Bytes::from_static(b"<svg width='1'/>"))
        .await
        .expect("tamper");
    let incomplete = (
        StatusCode::CONFLICT,
        json!({ "code": "version_incomplete" }),
    );
    assert_eq!(call(&app, Method::POST, &restore, None).await, incomplete);
    env.storage.storage.delete(&logo).await.expect("delete");
    let renders = env.mock.count();
    assert_eq!(call(&app, Method::POST, &restore, None).await, incomplete);
    assert_eq!(env.mock.count(), renders, "refused before the check");

    let (_, versions) = call(&app, Method::GET, "/api/pdf/design/versions", None).await;
    assert_eq!(versions.as_array().map(Vec::len), Some(2));
    let live = env.storage.bytes(&db.key("design/invoice.typ")).await;
    assert_eq!(live, Some(template("v2")), "live unchanged");
}
