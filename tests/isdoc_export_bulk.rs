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
