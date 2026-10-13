//! An unreachable storage: exports stop with 503, failed writes change
//! nothing, and cleanup never removes an object a row still points at.

mod common;

use axum::http::{Method, StatusCode};
use bytes::Bytes;
use common::documents::{create_doc, create_issued, dead_url, id, issuable, issue, set_company};
use common::isdoc::{confirm, fixture, zip_of};
use common::mdcast::{PdfEnv, get_raw};
use common::received::{create_received, upload};
use common::storage::dead_s3;
use common::{TestDb, call};
use invoiceapp::document::repo::original::remove_unreferenced;
use serde_json::{Value, json};

const PDF: &[u8] = b"%PDF-1.4\n% outage test\n%%EOF\n";

fn unavailable() -> Value {
    json!({ "code": "storage_unavailable" })
}

#[tokio::test]
async fn bulk_isdoc_export_stops_at_storage_unavailable() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let up = env.router(db.conn.clone());
    // An imported document without an original exports without the storage;
    // before the fix it made the bulk export a 200 with `errors.txt`.
    let base = issuable(&up).await;
    let mut imported = base.clone();
    imported["imported"] = json!(true);
    imported["number"] = json!("FV-2019/0042");
    let (status, _) = issue(&up, &id(&create_doc(&up, imported).await)).await;
    assert_eq!(status, StatusCode::OK);
    let native = create_issued(&up, base).await;

    let (status, _, _) = get_raw(&up, "/api/documents/isdoc").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "both export while the storage is up"
    );

    let down = env.router_with(db.conn.clone(), &env.url, dead_s3());
    let (status, err) = call(&down, Method::GET, "/api/documents/isdoc", None).await;
    assert_eq!(
        (status, err),
        (StatusCode::SERVICE_UNAVAILABLE, unavailable())
    );
    let uri = format!("/api/documents/{}/isdoc", id(&native));
    let (status, err) = call(&down, Method::GET, &uri, None).await;
    assert_eq!(
        (status, err),
        (StatusCode::SERVICE_UNAVAILABLE, unavailable())
    );
}

#[tokio::test]
async fn failed_original_upload_changes_nothing() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let up = env.router(db.conn.clone());
    set_company(&up, true).await;
    let s = common::documents::create_contact(&up, json!({ "name": "Dodavatel s.r.o." })).await;
    let doc_id = id(&create_received(&up, &s, json!({})).await);

    let down = env.router_with(db.conn.clone(), &env.url, dead_s3());
    let (status, err) = upload(&down, &doc_id, PDF).await;
    assert_eq!(
        (status, err),
        (StatusCode::SERVICE_UNAVAILABLE, unavailable())
    );
    let (_, doc) = call(&up, Method::GET, &format!("/api/documents/{doc_id}"), None).await;
    assert_eq!(doc["original"], Value::Null, "rolled back");
}

#[tokio::test]
async fn cleanup_keeps_objects_a_row_points_at() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    set_company(&app, true).await;
    let s = common::documents::create_contact(&app, json!({ "name": "Dodavatel s.r.o." })).await;
    let doc_id = id(&create_received(&app, &s, json!({})).await);
    let (status, _) = upload(&app, &doc_id, PDF).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Keys are relative to the space's storage (`spaces/{id}/…`).
    let pdf = common::space_pdf(&env.url, env.storage.storage.clone(), db.space);
    let storage = pdf.storage();
    let keys: Vec<String> = storage
        .list("documents")
        .await
        .expect("list")
        .into_iter()
        .map(|o| o.key)
        .collect();
    let [referenced] = keys.as_slice() else {
        panic!("one original stored: {keys:?}");
    };
    assert!(
        env.storage.bytes(&db.key(referenced)).await.is_some(),
        "stored under the space prefix"
    );
    let orphan = "documents/2026/orphan-original-00000000.pdf";
    storage
        .put(orphan, Bytes::from_static(PDF))
        .await
        .expect("put orphan");

    // The same content re-uploaded shares the key with the stored row: a
    // failure after that write must not delete the live original.
    remove_unreferenced(&db.conn, &pdf, referenced).await;
    remove_unreferenced(&db.conn, &pdf, orphan).await;
    assert_eq!(
        env.storage.bytes(&db.key(referenced)).await.as_deref(),
        Some(PDF)
    );
    assert_eq!(env.storage.bytes(&db.key(orphan)).await, None);
}

#[tokio::test]
async fn failed_isdoc_import_write_rolls_back() {
    use invoiceapp::document::repo::issue::Rate;
    use invoiceapp::error::AppError;
    use invoiceapp::import::store::{self, Options};
    use invoiceapp::isdoc::{parse, plan};

    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    set_company(&app, true).await;
    let pdf = common::space_pdf(&dead_url(), dead_s3(), db.space);
    let parsed = parse::parse(fixture("received_vat.isdoc").as_bytes()).expect("parsed");
    let p = plan::plan(parsed, Some("44444443")).expect("plan");
    let rate = Rate {
        rate: None,
        date: None,
        source: None,
    };
    let opts = Options {
        paid_on: Some(p.issue_date),
        category: None,
        vat_deductible: true,
    };
    let got = store::import(
        &db.conn,
        &pdf,
        &p,
        Some(Bytes::from_static(PDF)),
        &rate,
        &opts,
    )
    .await;
    assert!(
        matches!(got, Err(AppError::StorageUnavailable(_))),
        "{got:?}"
    );
    let (_, list) = call(&app, Method::GET, "/api/documents?direction=received", None).await;
    assert_eq!(list["total"], 0, "nothing imported");
}

#[tokio::test]
async fn isdoc_confirm_entry_fails_with_storage_unavailable() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let up = env.router(db.conn.clone());
    set_company(&up, true).await;
    let manifest = br#"<?xml version="1.0"?><manifest xmlns="http://isdoc.cz/namespace/2013/manifest"><maindocument filename="doc.isdoc"/></manifest>"#;
    let isdocx = zip_of(&[
        ("manifest.xml", manifest),
        ("doc.isdoc", fixture("received_vat.isdoc").as_bytes()),
        ("doc.pdf", PDF),
    ]);
    let files = [("doc.isdocx", isdocx.as_slice())];

    let down = env.router_with(db.conn.clone(), &env.url, dead_s3());
    let results = confirm(&down, &files, json!({ "selected": ["doc.isdocx"] })).await;
    assert_eq!(
        (&results[0]["status"], &results[0]["error"]),
        (&json!("failed"), &json!("storage_unavailable"))
    );
    let (_, list) = call(&up, Method::GET, "/api/documents?direction=received", None).await;
    assert_eq!(list["total"], 0, "nothing imported");
}
