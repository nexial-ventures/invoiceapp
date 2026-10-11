//! Issue renders outside any transaction (pdf.md, Archive): a slow mdcast
//! holds no row lock, and a change while rendering re-renders or conflicts.

mod common;

use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use common::auth::seed_space;
use common::documents::{create_doc, get_doc, id, issuable, issue};
use common::limits::used;
use common::mdcast::{PdfEnv, get_raw};
use common::members::{B_TOKEN, as_beta};
use common::{TestDb, call};
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};
use serde_json::{Value, json};

const RENDER: Duration = Duration::from_millis(1500);

fn spawn_issue(app: &Router, doc: &Value) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let (app, doc_id) = (app.clone(), id(doc));
    tokio::spawn(async move { issue(&app, &doc_id).await })
}

/// Lock `sql`'s rows without waiting: errors when another transaction holds them.
async fn lock_now(db: &DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
    let txn = db.begin().await?;
    let locked = txn
        .execute_unprepared(&format!("{sql} FOR UPDATE NOWAIT"))
        .await;
    txn.rollback().await?;
    locked.map(drop)
}

/// The draft PUT back with a header note (an edit bumps `updated_at`).
async fn edit(app: &Router, draft: &Value, body: &Value, note: &str) {
    let mut body = body.clone();
    body["headerNote"] = json!(note);
    body["dueDate"] = draft["dueDate"].clone();
    for k in [
        "currency",
        "locale",
        "vatMode",
        "paymentMethod",
        "bankAccountId",
        "taxPointDate",
    ] {
        body[k] = draft[k].clone();
    }
    let uri = format!("/api/documents/{}", id(draft));
    let (status, doc) = call(app, Method::PUT, &uri, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
}

fn archived(env: &PdfEnv, db: &TestDb) -> usize {
    std::fs::read_dir(env.storage.path().join(db.key("documents/2026")))
        .map(|d| d.count())
        .unwrap_or(0)
}

#[tokio::test]
async fn a_slow_render_holds_no_lock() {
    let db = TestDb::new().await;
    seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let beta = as_beta(&app);
    let body = issuable(&app).await;
    // One issue first, so the series' counter row exists to be locked.
    let (status, warm) = issue(&app, &id(&create_doc(&app, body.clone()).await)).await;
    assert_eq!(
        (status, &warm["number"]),
        (StatusCode::OK, &json!("20260001"))
    );
    let a = create_doc(&app, body.clone()).await;
    let c = create_doc(&app, body).await;
    let b = create_doc(&beta, issuable(&beta).await).await;
    env.mock.delay(RENDER);

    let first = spawn_issue(&app, &a);
    env.mock.wait_started(2).await;
    // Mid-render: the draft row and the number counter are free (rendering
    // inside the issue transaction held both until the commit).
    lock_now(
        &db.conn,
        &format!("SELECT 1 FROM documents WHERE id = '{}'", id(&a)),
    )
    .await
    .expect("document row not locked during the render");
    lock_now(&db.conn, "SELECT 1 FROM number_series_counters")
        .await
        .expect("number counter not locked during the render");
    // Another draft of the same space and one of another space issue meanwhile;
    // the second acme issue planned the number the first one takes.
    let second = spawn_issue(&app, &c);
    let other_space = spawn_issue(&beta, &b);

    let (s1, d1) = first.await.expect("task");
    let (s2, d2) = second.await.expect("task");
    let (s3, d3) = other_space.await.expect("task");
    assert_eq!(
        (s1, s2, s3),
        (StatusCode::OK, StatusCode::OK, StatusCode::OK)
    );
    assert_eq!(d1["number"], "20260002");
    assert_eq!(d2["number"], "20260003", "re-rendered with the next number");
    assert_eq!(d3["number"], "20260001", "beta has its own series");
    assert_eq!(env.mock.count(), 5, "one re-render");
    assert_eq!(
        archived(&env, &db),
        3,
        "the stale render's object is removed"
    );
}

#[tokio::test]
async fn racing_issues_of_one_series_all_get_consecutive_numbers() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let mut drafts = Vec::new();
    for _ in 0..3 {
        drafts.push(create_doc(&app, body.clone()).await);
    }
    env.mock.delay(Duration::from_millis(500));

    // All three plan 20260001; the losers re-render until they win a number
    // (the third loses twice — more than the edit budget allows).
    let tasks: Vec<_> = drafts.iter().map(|d| spawn_issue(&app, d)).collect();
    let (mut numbers, mut archive_bytes) = (Vec::new(), 0);
    for t in tasks {
        let (status, doc) = t.await.expect("task");
        assert_eq!(status, StatusCode::OK, "{doc}");
        numbers.push(doc["number"].as_str().expect("number").to_string());
        let (_, _, pdf) = get_raw(&app, &format!("/api/documents/{}/pdf", id(&doc))).await;
        archive_bytes += pdf.len() as u64;
    }
    numbers.sort();
    assert_eq!(numbers, ["20260001", "20260002", "20260003"]);
    assert_eq!(archived(&env, &db), 3, "lost renders removed");
    assert!(env.mock.count() > 3, "some issues re-rendered");
    assert_eq!(
        used(&app).await,
        archive_bytes,
        "only the committed archives count"
    );
}

#[tokio::test]
async fn a_lazy_archive_counts_once() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let (status, doc) = issue(&app, &id(&create_doc(&app, issuable(&app).await).await)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    // Like a DDPP after its payment, or a document from before archives.
    db.conn
        .execute_unprepared(
            "UPDATE documents SET pdf_path = NULL, pdf_sha256 = NULL, pdf_size = NULL",
        )
        .await
        .expect("drop the archive");
    assert_eq!(used(&app).await, 0);
    env.mock.delay(Duration::from_millis(300));

    // Two first downloads at once: both render, one archive is recorded.
    let uri = format!("/api/documents/{}/pdf", id(&doc));
    let (a, b) = tokio::join!(get_raw(&app, &uri), get_raw(&app, &uri));
    assert_eq!((a.0, b.0), (StatusCode::OK, StatusCode::OK));
    assert_eq!(a.2, b.2, "both serve the recorded archive");
    let size = a.2.len() as u64;
    assert_eq!(used(&app).await, size);
    let (_, _, again) = get_raw(&app, &uri).await;
    assert_eq!(again, a.2);
    assert_eq!(used(&app).await, size, "a later download adds nothing");
    assert_eq!(
        archived(&env, &db),
        2,
        "the dropped first archive + the recorded one: the losing render's object is removed"
    );
}

#[tokio::test]
async fn an_edit_during_the_render_is_issued_as_edited() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let draft = create_doc(&app, body.clone()).await;
    env.mock.delay(RENDER);

    let task = spawn_issue(&app, &draft);
    env.mock.wait_started(1).await;
    edit(&app, &draft, &body, "Edited while rendering").await;
    let (status, doc) = task.await.expect("task");
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["headerNote"], "Edited while rendering");
    assert_eq!(env.mock.count(), 2);
    assert_eq!(env.mock.last().data["headerNote"], "Edited while rendering");
    let (_, _, bytes) = get_raw(&app, &format!("/api/documents/{}/pdf", id(&doc))).await;
    assert_eq!(
        bytes,
        env.mock.last().pdf,
        "the archive is the second render"
    );
    assert_eq!(archived(&env, &db), 1);
}

#[tokio::test]
async fn edits_during_every_render_conflict_and_use_no_number() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let draft = create_doc(&app, body.clone()).await;
    env.mock.delay(RENDER);

    let task = spawn_issue(&app, &draft);
    env.mock.wait_started(1).await;
    edit(&app, &draft, &body, "first").await;
    env.mock.wait_started(2).await;
    edit(&app, &draft, &body, "second").await;
    let (status, err) = task.await.expect("task");
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err, json!({ "code": "conflict" }));
    let doc = get_doc(&app, &id(&draft)).await;
    assert_eq!(
        (&doc["status"], &doc["pdf"]),
        (&json!("draft"), &Value::Null)
    );
    assert_eq!(archived(&env, &db), 0, "both rendered objects removed");

    env.mock.delay(Duration::ZERO);
    let (status, doc) = issue(&app, &id(&draft)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["number"], "20260001", "no number was used");
}
