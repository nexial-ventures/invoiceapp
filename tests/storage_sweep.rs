//! The orphan sweep (pdf.md, Storage) on each backend: unreferenced
//! document objects of existing spaces older than the cutoff go; referenced
//! ones, young ones, other spaces' prefixes and every other key stay; a dry
//! run deletes nothing; the guard refuses an empty reference set and too
//! many orphans unless forced.

mod common;

use axum::http::StatusCode;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use common::TestDb;
use common::documents::{archive_rel, create_doc, id, issuable, issue};
use common::mdcast::PdfEnv;
use common::storage::TestStorage;
use invoice::storage::Storage;
use invoice::storage::sweep::{self, MAX_ORPHANS, Options, Swept};

macro_rules! suite {
    ($($case:ident),* $(,)?) => {
        mod fs {
            $(#[tokio::test]
            async fn $case() {
                super::$case(super::TestStorage::fs()).await;
            })*
        }
        mod s3 {
            $(#[tokio::test]
            async fn $case() {
                super::$case(super::TestStorage::s3()).await;
            })*
        }
    };
}

suite!(sweeps_orphans, refuses_without_references);

/// Every object counts as old (S3 clocks may differ from ours).
fn later() -> DateTime<Utc> {
    Utc::now() + chrono::Duration::minutes(10)
}

async fn run(db: &TestDb, s: &Storage, cutoff: DateTime<Utc>, dry_run: bool, force: bool) -> Swept {
    let opts = Options {
        cutoff,
        dry_run,
        force,
    };
    sweep::sweep(&db.conn, s, opts).await.expect("sweep")
}

async fn put(s: &Storage, key: &str) {
    s.put(key, Bytes::from_static(b"%PDF-orphan"))
        .await
        .expect("put");
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
    let kept = [
        // A space that does not exist (here: another test's) is never touched.
        format!("spaces/{}/documents/2026/x.pdf", uuid::Uuid::new_v4()),
        db.key("design/logo.svg"),
        db.key("email/templates/cs.json"),
        "documents/2026/outside-spaces.pdf".to_string(),
    ];
    put(s, &orphan).await;
    for key in &kept {
        put(s, key).await;
    }

    // Everything was just written: nothing is old enough at the real cutoff.
    let young = run(&db, s, sweep::cutoff_now().expect("cutoff"), false, false).await;
    assert_eq!((young.listed, young.young), (2, 1), "{young:?}");
    assert!(young.orphans.is_empty() && young.refused.is_none());

    let dry = run(&db, s, later(), true, false).await;
    assert_eq!(dry.orphans, [orphan.clone()]);
    assert!(dry.deleted.is_empty());
    assert!(
        env.storage.bytes(&orphan).await.is_some(),
        "dry run deletes nothing"
    );

    let swept = run(&db, s, later(), false, false).await;
    assert_eq!(
        (swept.listed, &swept.deleted, &swept.failed, &swept.refused),
        (2, &vec![orphan.clone()], &vec![], &None)
    );
    assert!(env.storage.bytes(&orphan).await.is_none());
    assert!(env.storage.bytes(&archive).await.is_some(), "referenced");
    for key in &kept {
        assert!(env.storage.bytes(key).await.is_some(), "{key} untouched");
    }
    let again = run(&db, s, later(), false, false).await;
    assert_eq!((again.listed, again.deleted.len()), (1, 0));

    // Too many orphans: refused unless forced.
    let many: Vec<String> = (0..=MAX_ORPHANS)
        .map(|n| db.key(&format!("documents/2026/orphan-{n:03}.pdf")))
        .collect();
    futures_util::future::join_all(many.iter().map(|k| put(s, k))).await;
    let refused = run(&db, s, later(), false, false).await;
    assert!(
        refused
            .refused
            .as_deref()
            .is_some_and(|r| r.contains("exceed the limit of 100")),
        "{refused:?}"
    );
    assert_eq!(
        (refused.orphans.len(), refused.deleted.len()),
        (MAX_ORPHANS + 1, 0)
    );
    assert!(env.storage.bytes(&many[0]).await.is_some());
    let forced = run(&db, s, later(), false, true).await;
    assert_eq!(forced.deleted.len(), MAX_ORPHANS + 1);
    assert!(env.storage.bytes(&archive).await.is_some());
}

/// No `documents` row references anything (an empty or wrong database):
/// nothing is deleted unless forced.
async fn refuses_without_references(storage: TestStorage) {
    let db = TestDb::new().await;
    let s = &storage.storage;
    let orphan = db.key("documents/2026/lost.pdf");
    put(s, &orphan).await;

    let refused = run(&db, s, later(), false, false).await;
    assert!(
        refused
            .refused
            .as_deref()
            .is_some_and(|r| r.contains("no document references")),
        "{refused:?}"
    );
    assert!(refused.deleted.is_empty());
    assert!(storage.bytes(&orphan).await.is_some());

    let forced = run(&db, s, later(), false, true).await;
    assert_eq!(forced.deleted, [orphan.clone()]);
    assert!(storage.bytes(&orphan).await.is_none());
}
