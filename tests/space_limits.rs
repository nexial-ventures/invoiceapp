//! Hidden per-space limits (spaces.md, "Limits"): the row every space gets,
//! the defaults without one, the storage usage and the quota on original
//! uploads; issuing is never blocked; spaces don't share any of it.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, on, seed_space};
use common::documents::{create_contact, create_doc, id, issuable, issue, set_company};
use common::limits::{KIB, MIB, limits, pdf_of, set_quota, used};
use common::mdcast::PdfEnv;
use common::members::{B_HOST, B_TOKEN, count};
use common::received::{create_received, upload};
use common::{TestDb, call};
use invoice::migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectOptions, ConnectionTrait, Database};
use serde_json::json;

const DEFAULT_QUOTA: u64 = 100 * MIB;

#[tokio::test]
async fn every_space_gets_a_limits_row_and_missing_means_defaults() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", None).await;
    for space in [db.space, beta] {
        let sql = format!(
            "SELECT count(*) FROM space_limits WHERE space_id = '{space}' \
             AND storage_quota_bytes = {DEFAULT_QUOTA}"
        );
        assert_eq!(count(&db.conn, &sql).await, 1, "{space}");
    }
    assert_eq!(
        limits(&app).await,
        json!({ "storageQuotaBytes": DEFAULT_QUOTA, "storageUsedBytes": 0 })
    );
    // `PUT /api/space` answers the same shape.
    let (status, body) = call(
        &app,
        Method::PUT,
        "/api/space",
        Some(json!({ "name": "Acme" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["limits"]["storageQuotaBytes"], DEFAULT_QUOTA);

    db.conn
        .execute_unprepared(&format!(
            "DELETE FROM space_limits WHERE space_id = '{}'",
            db.space
        ))
        .await
        .expect("drop the row");
    assert_eq!(limits(&app).await["storageQuotaBytes"], DEFAULT_QUOTA);
    // The defaults also enforce: 100 MiB is far away.
    set_company(&app, true).await;
    let s = create_contact(&app, json!({ "name": "Dodavatel s.r.o." })).await;
    let doc = id(&create_received(&app, &s, json!({})).await);
    let (status, body) = upload(&app, &doc, &pdf_of(KIB)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(used(&app).await, KIB as u64);
}

#[tokio::test]
async fn original_uploads_respect_the_quota_and_deletes_free_it() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    set_company(&app, true).await;
    let s = create_contact(&app, json!({ "name": "Dodavatel s.r.o." })).await;
    let first = id(&create_received(&app, &s, json!({})).await);
    let second = id(&create_received(&app, &s, json!({})).await);
    set_quota(&db.conn, db.space, MIB).await;

    let (status, body) = upload(&app, &first, &pdf_of(600 * KIB)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(used(&app).await, 600 * KIB as u64);
    let (status, body) = upload(&app, &second, &pdf_of(500 * KIB)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({ "code": "quota_exceeded", "detail": "0.6 / 1 MB" })
    );
    let (_, d) = call(&app, Method::GET, &format!("/api/documents/{second}"), None).await;
    assert_eq!(d["original"], serde_json::Value::Null, "nothing stored");
    // The object put before the transaction was removed with the refusal.
    let keys = env.storage.keys(&db.key("documents")).await;
    assert!(!keys.iter().any(|k| k.contains(&second)), "{keys:?}");
    // Replacing counts only the difference: 600 → 1000 KiB fits 1 MiB.
    let (status, body) = upload(&app, &first, &pdf_of(1000 * KIB)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(used(&app).await, 1000 * KIB as u64);

    // The operator lowers the quota below the usage: growing is refused,
    // shrinking still works.
    set_quota(&db.conn, db.space, 100 * KIB as u64).await;
    let (status, _) = upload(&app, &first, &pdf_of(1001 * KIB)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, body) = upload(&app, &first, &pdf_of(900 * KIB)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(used(&app).await, 900 * KIB as u64);

    // Deleting the original, then the document, frees the bytes.
    set_quota(&db.conn, db.space, MIB).await;
    let uri = format!("/api/documents/{first}/original");
    let (status, _) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(used(&app).await, 0);
    let (status, body) = upload(&app, &second, &pdf_of(700 * KIB)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(used(&app).await, 700 * KIB as u64);
    let uri = format!("/api/documents/{second}");
    let (status, _) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(used(&app).await, 0);
}

#[tokio::test]
async fn issuing_counts_the_archive_but_is_never_blocked() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = id(&create_doc(&app, body).await);
    set_quota(&db.conn, db.space, 0).await;
    let (status, issued) = issue(&app, &doc).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let archived = count(
        &db.conn,
        &format!("SELECT pdf_size FROM documents WHERE id = '{doc}'"),
    )
    .await;
    assert!(archived > 0, "the archive size is recorded");
    assert_eq!(used(&app).await, archived as u64);
    assert_eq!(limits(&app).await["storageQuotaBytes"], 0);

    // Over the quota now: an original upload is refused.
    let s = create_contact(
        &app,
        json!({ "name": "Dodavatel s.r.o.", "ico": "27074358" }),
    )
    .await;
    let received = id(&create_received(&app, &s, json!({})).await);
    let (status, body) = upload(&app, &received, &pdf_of(KIB)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "quota_exceeded");
}

#[tokio::test]
async fn limits_and_usage_are_per_space() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    set_company(&app, true).await;
    let s = create_contact(&app, json!({ "name": "Dodavatel s.r.o." })).await;
    let doc = id(&create_received(&app, &s, json!({})).await);
    let (status, _) = upload(&app, &doc, &pdf_of(300 * KIB)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    set_quota(&db.conn, db.space, 0).await;
    set_quota(&db.conn, beta, 5 * MIB).await;

    let beta_space = || {
        on(
            &app,
            Method::GET,
            B_HOST,
            "/api/space",
            As::Bearer(B_TOKEN),
            None,
        )
    };
    let (status, body) = beta_space().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["limits"],
        json!({ "storageQuotaBytes": 5 * MIB, "storageUsedBytes": 0 })
    );
    assert_eq!(
        limits(&app).await,
        json!({ "storageQuotaBytes": 0, "storageUsedBytes": 300 * KIB })
    );
    // Deleting a space removes its limits row.
    db.conn
        .execute_unprepared(&format!("DELETE FROM spaces WHERE id = '{beta}'"))
        .await
        .expect("delete beta");
    let sql = format!("SELECT count(*) FROM space_limits WHERE space_id = '{beta}'");
    assert_eq!(count(&db.conn, &sql).await, 0);
}

#[tokio::test]
async fn the_migration_creates_rows_for_existing_spaces() {
    let url = common::test_database_url();
    let schema = common::unique_schema();
    let admin = Database::connect(&url).await.expect("connect");
    admin
        .execute_unprepared(&format!("CREATE SCHEMA \"{schema}\""))
        .await
        .expect("schema");
    let mut opts = ConnectOptions::new(url.clone());
    opts.set_schema_search_path(schema.clone())
        .max_connections(2)
        .sqlx_logging(false);
    let conn = Database::connect(opts).await.expect("connect schema");
    // Everything before the limits migration, with one space.
    let before = Migrator::migrations().len() as u32 - 1;
    Migrator::up(&conn, Some(before)).await.expect("migrate");
    conn.execute_unprepared(
        "INSERT INTO spaces (id, slug, name) VALUES (gen_random_uuid(), 'old', 'Old')",
    )
    .await
    .expect("old space");
    Migrator::up(&conn, None).await.expect("limits migration");
    let sql = "SELECT count(*) FROM space_limits l JOIN spaces s ON s.id = l.space_id \
               WHERE s.slug = 'old' AND l.storage_quota_bytes = 104857600";
    assert_eq!(count(&conn, sql).await, 1);
    conn.close().await.expect("close");
    admin
        .execute_unprepared(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .await
        .expect("drop schema");
}
