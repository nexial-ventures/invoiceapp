//! ISDOC bulk export: list order with concurrent exports, and which entries
//! are stored uncompressed.

mod common;

use axum::http::StatusCode;
use common::TestDb;
use common::documents::{create_issued, issuable};
use common::isdoc::unzip;
use common::mdcast::{PdfEnv, get_raw};

fn entry<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> &'a [u8] {
    &files
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .1
}

/// Entry names and compression methods, in archive order.
fn methods(bytes: &[u8]) -> Vec<(String, zip::CompressionMethod)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    (0..z.len())
        .map(|i| {
            let e = z.by_index_raw(i).expect("entry");
            (e.name().expect("name").into_owned(), e.compression())
        })
        .collect()
}

#[tokio::test]
async fn bulk_export_keeps_list_order_and_stores_compressed_payloads() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let base = issuable(&app).await;
    // More documents than are exported at once.
    let mut numbers = Vec::new();
    for _ in 0..11 {
        let d = create_issued(&app, base.clone()).await;
        numbers.push(format!("{}.isdocx", d["number"].as_str().expect("number")));
    }
    let (status, _, bytes) = get_raw(&app, "/api/documents/isdoc").await;
    assert_eq!(status, StatusCode::OK);
    let entries = methods(&bytes);
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, numbers, "issue date, then number");
    assert!(
        entries
            .iter()
            .all(|(_, m)| *m == zip::CompressionMethod::Stored),
        "an .isdocx is a zip already: {entries:?}"
    );
    let isdocx = entry(&unzip(&bytes), &numbers[0]).to_vec();
    let inner: std::collections::HashMap<_, _> = methods(&isdocx).into_iter().collect();
    let stem = numbers[0].trim_end_matches(".isdocx");
    assert_eq!(
        inner[&format!("{stem}.pdf")],
        zip::CompressionMethod::Stored
    );
    assert_eq!(
        inner[&format!("{stem}.isdoc")],
        zip::CompressionMethod::Deflated
    );
    assert_eq!(inner["manifest.xml"], zip::CompressionMethod::Deflated);
}

async fn archived_count(db: &TestDb) -> Option<i64> {
    use sea_orm::ConnectionTrait;
    db.conn
        .query_one(sea_orm::Statement::from_string(
            db.conn.get_database_backend(),
            "SELECT count(*) AS n FROM documents WHERE pdf_path IS NOT NULL",
        ))
        .await
        .expect("count")
        .and_then(|r| r.try_get::<i64>("", "n").ok())
}

#[tokio::test]
async fn a_dropped_request_still_records_the_lazy_archive() {
    use sea_orm::ConnectionTrait;
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    create_issued(&app, issuable(&app).await).await;
    db.conn
        .execute_unprepared(
            "UPDATE documents SET pdf_path = NULL, pdf_sha256 = NULL, pdf_size = NULL",
        )
        .await
        .expect("drop the archive");
    env.mock.delay(std::time::Duration::from_millis(300));
    let started = env.mock.started();
    let request = tokio::spawn({
        let app = app.clone();
        async move { get_raw(&app, "/api/documents/isdoc").await }
    });
    env.mock.wait_started(started + 1).await;
    request.abort();
    let mut recorded = false;
    for _ in 0..300 {
        if archived_count(&db).await == Some(1) {
            recorded = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        recorded,
        "the render finished and was recorded after the abort"
    );
}
