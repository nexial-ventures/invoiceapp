//! The orphan sweep (pdf.md, Storage) on each backend: unreferenced
//! `spaces/*/documents/` objects older than the cutoff go, referenced ones,
//! young ones and every other key stay; a dry run deletes nothing.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use bytes::Bytes;
use chrono::Utc;
use common::TestDb;
use common::documents::{archive_rel, create_doc, id, issuable, issue};
use common::mdcast::PdfEnv;
use common::storage::TestStorage;
use invoice::storage::sweep::{self, MIN_AGE};

#[tokio::test]
async fn sweep_fs() {
    sweeps_orphans(TestStorage::fs()).await;
}

#[tokio::test]
async fn sweep_s3() {
    sweeps_orphans(TestStorage::s3()).await;
}

async fn sweeps_orphans(storage: TestStorage) {
    let db = TestDb::new().await;
    let env = PdfEnv::with_storage(storage);
    let app = env.router(db.conn.clone());
    let s = &env.storage.storage;

    let (status, doc) = issue(&app, &id(&create_doc(&app, issuable(&app).await).await)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let archive = db.key(&archive_rel(&db.conn, &doc).await);
    let orphan =
        db.key("documents/2026/00000000-0000-0000-0000-000000000000-deadbeef-0a1b2c3d.pdf");
    let gone_space = format!("spaces/{}/documents/2026/x.pdf", uuid::Uuid::new_v4());
    let others = [
        db.key("design/logo.svg"),
        db.key("email/templates/cs.json"),
        "documents/2026/outside-spaces.pdf".to_string(),
    ];
    for key in [&orphan, &gone_space].into_iter().chain(&others) {
        s.put(key, Bytes::from_static(b"%PDF-orphan"))
            .await
            .expect("put");
    }
    let mut expected = vec![orphan.clone(), gone_space.clone()];
    expected.sort();

    // Everything was just written: nothing is old enough at the real cutoff.
    let young = sweep::sweep_now(&db.conn, s, false).await.expect("sweep");
    assert_eq!((young.listed, young.young), (3, 2), "{young:?}");
    assert!(young.orphans.is_empty());
    assert!(MIN_AGE >= Duration::from_secs(3600));

    // A cutoff in the future makes every object old (S3 clocks may differ).
    let later = Utc::now() + chrono::Duration::minutes(10);
    let dry = sweep::sweep(&db.conn, s, later, true)
        .await
        .expect("dry run");
    assert_eq!((dry.orphans.clone(), dry.deleted), (expected.clone(), 0));
    assert!(
        env.storage.bytes(&orphan).await.is_some(),
        "dry run deletes nothing"
    );

    let swept = sweep::sweep(&db.conn, s, later, false)
        .await
        .expect("sweep");
    assert_eq!(
        (
            swept.listed,
            swept.orphans.clone(),
            swept.deleted,
            swept.failed
        ),
        (3, expected, 2, 0)
    );
    assert!(env.storage.bytes(&orphan).await.is_none());
    assert!(env.storage.bytes(&gone_space).await.is_none());
    assert!(env.storage.bytes(&archive).await.is_some(), "referenced");
    for key in &others {
        assert!(env.storage.bytes(key).await.is_some(), "{key} untouched");
    }

    let again = sweep::sweep(&db.conn, s, later, false)
        .await
        .expect("sweep");
    assert_eq!((again.listed, again.deleted), (1, 0));
}
