//! The space storage quota on ISDOC imports carrying PDFs and on design
//! uploads (spaces.md, "Limits"; design.md keeps its own 50 MB on top).

mod common;

use axum::http::{Method, StatusCode};
use common::design::{listing, upload, upload_ok};
use common::documents::set_company;
use common::isdoc::{confirm, fixture, form, post_form, zip_of};
use common::limits::{KIB, MIB, pdf_of, set_quota, used};
use common::mdcast::PdfEnv;
use common::members::count;
use common::received::{create_received, upload as upload_original};
use common::{TestDb, call};
use serde_json::json;

const MANIFEST: &[u8] = br#"<?xml version="1.0"?><manifest xmlns="http://isdoc.cz/namespace/2013/manifest"><maindocument filename="doc.isdoc"/></manifest>"#;

/// An `.isdocx` of the received fixture (supplier number `number`) with a PDF.
fn isdocx(number: &str, pdf: &[u8]) -> Vec<u8> {
    let xml = fixture("received_vat.isdoc")
        .replace("<ID>FV-2026/077</ID>", &format!("<ID>{number}</ID>"));
    zip_of(&[
        ("manifest.xml", MANIFEST),
        ("doc.isdoc", xml.as_bytes()),
        ("doc.pdf", pdf),
    ])
}

#[tokio::test]
async fn isdoc_confirm_refuses_pdfs_over_the_quota() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    set_company(&app, true).await;
    set_quota(&db.conn, db.space, MIB).await;
    let (a, b) = (
        isdocx("A-1", &pdf_of(600 * KIB)),
        isdocx("B-2", &pdf_of(600 * KIB)),
    );
    let files = [("a.isdocx", a.as_slice()), ("b.isdocx", b.as_slice())];
    let options = json!({ "selected": ["a.isdocx", "b.isdocx"] }).to_string();

    // Both together do not fit: nothing is imported.
    let uri = "/api/import/isdoc/confirm";
    let (status, body) = post_form(&app, uri, form(&files, Some(&options))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        body,
        json!({ "code": "quota_exceeded", "detail": "0.0 / 1 MB" })
    );
    let docs = format!(
        "SELECT count(*) FROM documents WHERE space_id = '{}'",
        db.space
    );
    assert_eq!(count(&db.conn, &docs).await, 0);

    // One fits, and then counts.
    let results = confirm(&app, &files[..1], json!({ "selected": ["a.isdocx"] })).await;
    assert_eq!(results[0]["status"], "imported", "{results:?}");
    assert_eq!(used(&app).await, 600 * KIB as u64);
    let (status, body) = post_form(
        &app,
        uri,
        form(
            &files[1..],
            Some(&json!({ "selected": ["b.isdocx"] }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["detail"], "0.6 / 1 MB");

    // A document without a PDF adds nothing and still imports.
    let plain = fixture("received_vat.isdoc").replace("<ID>FV-2026/077</ID>", "<ID>C-3</ID>");
    let plain = [("c.isdoc", plain.as_bytes())];
    let results = confirm(&app, &plain, json!({ "selected": ["c.isdoc"] })).await;
    assert_eq!(results[0]["status"], "imported", "{results:?}");
}

/// The space's quota lock, held until the returned transaction ends.
async fn quota_lock(db: &TestDb) -> sea_orm::DatabaseTransaction {
    use sea_orm::{ConnectionTrait, TransactionTrait};
    let txn = db.conn.begin().await.expect("begin");
    txn.execute_unprepared(&format!(
        "SELECT pg_advisory_xact_lock(hashtextextended('space-quota:{}', 0))",
        db.space
    ))
    .await
    .expect("take the quota lock");
    txn
}

async fn wait_for_lock_waiter(db: &TestDb) {
    let waiting = "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted";
    for _ in 0..500 {
        if count(&db.conn, waiting).await > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("nobody waited on the quota lock");
}

#[tokio::test]
async fn an_entry_refused_by_the_quota_leaves_no_object() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    set_company(&app, true).await;
    set_quota(&db.conn, db.space, MIB).await;
    let a = isdocx("A-1", &pdf_of(600 * KIB));
    let files = [("a.isdocx", a.as_slice())];

    // The batch check passes; the quota shrinks while the entry, its PDF
    // already put, waits for the quota lock.
    let held = quota_lock(&db).await;
    let task = {
        let app = app.clone();
        let form = form(
            &files,
            Some(&json!({ "selected": ["a.isdocx"] }).to_string()),
        );
        tokio::spawn(async move { post_form(&app, "/api/import/isdoc/confirm", form).await })
    };
    wait_for_lock_waiter(&db).await;
    set_quota(&db.conn, db.space, 100 * KIB as u64).await;
    held.commit().await.expect("release the quota lock");

    let (status, body) = task.await.expect("task");
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.to_string().contains("quota_exceeded"), "{body}");
    let docs = format!(
        "SELECT count(*) FROM documents WHERE space_id = '{}'",
        db.space
    );
    assert_eq!(count(&db.conn, &docs).await, 0);
    assert_eq!(used(&app).await, 0);
    assert!(
        env.storage.keys(&db.key("documents")).await.is_empty(),
        "the entry's PDF was removed"
    );
}

#[tokio::test]
async fn design_uploads_count_toward_the_space_quota() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    set_quota(&db.conn, db.space, MIB).await;

    upload_ok(&app, "a.png", pdf_of(700 * KIB)).await;
    assert_eq!(
        used(&app).await,
        700 * KIB as u64,
        "recorded by the listing"
    );
    let (status, body) = upload(&app, "b.png", pdf_of(400 * KIB)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        body,
        json!({ "code": "quota_exceeded", "detail": "0.7 / 1 MB" })
    );
    // Replacing the draft file counts the difference only.
    upload_ok(&app, "a.png", pdf_of(1000 * KIB)).await;

    // Activation copies into live and a version: counted, never blocked.
    upload_ok(&app, "a.png", pdf_of(300 * KIB)).await;
    let (status, body) = call(&app, Method::POST, "/api/pdf/design/activate", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(used(&app).await, 600 * KIB as u64, "live + version");

    // Documents count against design uploads too.
    set_company(&app, true).await;
    let s = common::documents::create_contact(&app, json!({ "name": "D s.r.o." })).await;
    let doc = common::documents::id(&create_received(&app, &s, json!({})).await);
    let (status, body) = upload_original(&app, &doc, &pdf_of(300 * KIB)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = upload(&app, "c.png", pdf_of(200 * KIB)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["detail"], "0.9 / 1 MB");

    // A reset deletes the live files (the version keeps its copy).
    let (status, _) = call(&app, Method::POST, "/api/pdf/design/reset", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        used(&app).await,
        600 * KIB as u64,
        "300 original + 300 version"
    );
    assert_eq!(listing(&app).await["quota"]["used"], 300 * KIB);
}
