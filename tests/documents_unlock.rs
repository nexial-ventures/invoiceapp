//! `POST /api/documents/{id}/unlock`: issued → draft with the number kept,
//! the archive removed, who / when / why recorded; role, state and reason
//! rules; space isolation.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, member, on, seed_space};
use common::documents::{
    archive_rel, create_doc, create_issued, get_doc, id, issuable, post_action, unlock,
};
use common::mdcast::{PdfEnv, get_raw};
use common::members::{B_TOKEN, as_beta};
use common::received::create_received;
use common::{OWNER_EMAIL, TEST_HOST, TestDb, call, router};
use invoiceapp::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

#[tokio::test]
async fn unlock_returns_a_numbered_draft_and_removes_the_archive() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    let archive = env
        .storage
        .path()
        .join(db.key(&archive_rel(&db.conn, &doc).await));
    assert!(archive.exists());
    let (status, sent) = post_action(&app, &id(&doc), "mark-sent").await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    let (status, draft) = unlock(&app, &id(&doc), json!("  Wrong VAT rate  ")).await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    assert_eq!(
        (&draft["status"], &draft["number"], &draft["paymentState"]),
        (&json!("draft"), &json!("20260001"), &Value::Null)
    );
    assert_eq!(draft["sentAt"], sent["sentAt"], "sentAt is kept");
    assert_eq!(draft["unlockReason"], "Wrong VAT rate");
    assert!(draft["unlockedAt"].is_string());
    assert_eq!(draft["unlockedBy"]["email"], OWNER_EMAIL);
    assert!(draft["unlockedBy"]["displayName"].is_string());
    assert_eq!(
        (&draft["pdf"], &draft["supplier"]),
        (&Value::Null, &Value::Null)
    );
    assert!(!archive.exists(), "the rendered archive is removed");

    // The list shows the unlock too.
    let (_, list) = call(&app, Method::GET, "/api/documents", None).await;
    let row = &list["items"][0];
    assert_eq!(
        (
            &row["status"],
            &row["unlockReason"],
            &row["unlockedBy"]["email"],
            &row["hasPdf"]
        ),
        (
            &json!("draft"),
            &json!("Wrong VAT rate"),
            &json!(OWNER_EMAIL),
            &json!(false)
        )
    );
}

#[tokio::test]
async fn unlock_needs_the_manage_role() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    let uri = format!("/api/documents/{}/unlock", id(&doc));
    let reason = Some(json!({ "reason": "typo" }));

    let writer = member(&db.conn, db.space, "member@example.com", Role::Member).await;
    let (status, err) = on(
        &app,
        Method::POST,
        TEST_HOST,
        &uri,
        As::Bearer(&writer),
        reason.clone(),
    )
    .await;
    assert_eq!(
        (status, err),
        (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }))
    );
    assert_eq!(get_doc(&app, &id(&doc)).await["status"], "issued");

    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let (status, draft) = on(
        &app,
        Method::POST,
        TEST_HOST,
        &uri,
        As::Bearer(&admin),
        reason,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    assert_eq!(draft["unlockedBy"]["email"], "admin@example.com");
}

#[tokio::test]
async fn only_issued_documents_of_ours_unlock() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let conflict = (StatusCode::CONFLICT, json!({ "code": "invalid_state" }));

    let draft = create_doc(&app, body.clone()).await;
    assert_eq!(unlock(&app, &id(&draft), json!("x")).await, conflict);

    let cancelled = create_issued(&app, body.clone()).await;
    let (status, _) = post_action(&app, &id(&cancelled), "cancel").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unlock(&app, &id(&cancelled), json!("x")).await, conflict);

    let contact = body["contactId"].as_str().expect("contact").to_string();
    let received = create_received(&app, &contact, json!({})).await;
    assert_eq!(unlock(&app, &id(&received), json!("x")).await, conflict);

    // Twice: the second finds a draft.
    let doc = create_issued(&app, body).await;
    assert_eq!(unlock(&app, &id(&doc), json!("x")).await.0, StatusCode::OK);
    assert_eq!(unlock(&app, &id(&doc), json!("x")).await, conflict);
}

#[tokio::test]
async fn the_reason_is_required_and_bounded() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    let field = |reason: &str| json!({ "code": "validation", "fields": { "reason": reason } });
    let uri = format!("/api/documents/{}/unlock", id(&doc));

    let (status, err) = call(&app, Method::POST, &uri, None).await;
    assert_eq!(
        (status, err),
        (StatusCode::UNPROCESSABLE_ENTITY, field("required"))
    );
    for reason in [Value::Null, json!(""), json!("   ")] {
        let (status, err) = unlock(&app, &id(&doc), reason).await;
        assert_eq!(
            (status, err),
            (StatusCode::UNPROCESSABLE_ENTITY, field("required"))
        );
    }
    let (status, err) = unlock(&app, &id(&doc), json!("é".repeat(2001))).await;
    assert_eq!(
        (status, err),
        (StatusCode::UNPROCESSABLE_ENTITY, field("too_long"))
    );
    assert_eq!(get_doc(&app, &id(&doc)).await["status"], "issued");

    let (status, draft) = unlock(&app, &id(&doc), json!("é".repeat(2000))).await;
    assert_eq!(status, StatusCode::OK, "{draft}");
}

#[tokio::test]
async fn another_space_cannot_unlock() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    let beta = as_beta(&app);

    let (status, err) = unlock(&beta, &id(&doc), json!("mine now")).await;
    assert_eq!(
        (status, err),
        (StatusCode::NOT_FOUND, json!({ "code": "not_found" }))
    );
    assert_eq!(get_doc(&app, &id(&doc)).await["status"], "issued");
}

#[tokio::test]
async fn a_render_overtaken_by_an_unlock_is_not_archived() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    // No archive yet (like a DDPP after its payment): the first download renders one.
    db.conn
        .execute_unprepared(
            "UPDATE documents SET pdf_path = NULL, pdf_sha256 = NULL, pdf_size = NULL",
        )
        .await
        .expect("drop the archive");
    let dir = env.storage.path().join(db.key("documents/2026"));
    let objects = || std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
    let before = objects();

    let started = env.mock.started();
    env.mock.hold();
    let uri = format!("/api/documents/{}/pdf", id(&doc));
    let task = {
        let (app, uri) = (app.clone(), uri.clone());
        tokio::spawn(async move { get_raw(&app, &uri).await })
    };
    env.mock.wait_started(started + 1).await;
    let (status, _) = unlock(&app, &id(&doc), json!("while rendering")).await;
    assert_eq!(status, StatusCode::OK);
    env.mock.release();

    let (status, _, body) = task.await.expect("download task");
    let shown = String::from_utf8_lossy(&body);
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "never a draft render: {shown}"
    );
    let draft = get_doc(&app, &id(&doc)).await;
    assert_eq!(
        (&draft["status"], &draft["pdf"]),
        (&json!("draft"), &Value::Null)
    );
    assert_eq!(
        objects(),
        before,
        "the overtaken render's object is removed"
    );
}
