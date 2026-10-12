//! DDPP archives, preview, design listing / override, auth.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{
    create_doc, create_issued, dead_url, get_doc, id, issuable, issue, item, pay,
};
use common::mdcast::{PdfEnv, get_raw};
use common::{TestDb, call, get, router, send};
use serde_json::{Value, json};

async fn proforma(app: &axum::Router) -> Value {
    let mut body = issuable(app).await;
    body["docType"] = json!("proforma");
    body["lines"] = json!([item("1", "1000", "21")]);
    create_issued(app, body).await
}

#[tokio::test]
async fn ddpp_payment_survives_mdcast_down_and_first_download_archives() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let up = env.router(db.conn.clone());
    let p = proforma(&up).await;

    let down = env.router_at(db.conn.clone(), &dead_url());
    let (status, payment) = pay(&down, &id(&p), json!({ "amount": "1210" })).await;
    assert_eq!(status, StatusCode::CREATED, "{payment}");
    let ddpp = payment["advanceDocumentId"]
        .as_str()
        .expect("DDPP")
        .to_string();
    assert_eq!(get_doc(&up, &ddpp).await["pdf"], Value::Null);

    let uri = format!("/api/documents/{ddpp}/pdf");
    let (status, headers, bytes) = get_raw(&up, &uri).await;
    assert_eq!(status, StatusCode::OK);
    let render = env.mock.last();
    assert_eq!(bytes, render.pdf);
    assert_eq!(render.data["docType"], "advance_tax_doc");
    assert_eq!(render.data["paidNote"], "Neplaťte – již uhrazeno.");
    assert_eq!(render.data["reference"], "K zálohové faktuře Z20260001");
    assert_eq!(render.data["qr"], Value::Null);
    assert_eq!(
        headers["content-disposition"],
        "inline; filename=\"DP20260001.pdf\""
    );
    // The browser's PDF viewer works under the CSP: no `sandbox` on it.
    assert_eq!(
        headers["content-security-policy"],
        invoice::security_headers::CSP
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
    let doc = get_doc(&up, &ddpp).await;
    assert_eq!(
        doc["pdf"]["sha256"],
        json!(invoice::storage::sha256_hex(&bytes))
    );

    // From now on the archive is served.
    let renders = env.mock.count();
    let (_, _, again) = get_raw(&up, &uri).await;
    assert_eq!((again, env.mock.count()), (bytes, renders));
}

#[tokio::test]
async fn ddpp_is_archived_right_after_the_payment() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let p = proforma(&app).await;
    let (status, payment) = pay(&app, &id(&p), json!({ "amount": "605" })).await;
    assert_eq!(status, StatusCode::CREATED, "{payment}");
    let ddpp = payment["advanceDocumentId"].as_str().expect("DDPP");
    // Archived in the background after the 201.
    let mut doc = get_doc(&app, ddpp).await;
    for _ in 0..100 {
        if doc["pdf"]["sha256"].is_string() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        doc = get_doc(&app, ddpp).await;
    }
    assert!(doc["pdf"]["sha256"].is_string(), "{doc}");
    assert_eq!(env.mock.count(), 2, "proforma + DDPP rendered once each");
    assert!(
        env.storage
            .path()
            .join(db.key(&common::documents::archive_rel(&db.conn, &doc).await))
            .is_file()
    );
}

#[tokio::test]
async fn default_design_listing() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (status, listing) = call(&app, Method::GET, "/api/pdf/design", None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert_eq!(listing["storage"], "fs");
    let files = listing["files"].as_array().expect("files");
    assert!(files.iter().all(|f| f["source"] == "default"));
    assert!(files.iter().any(|f| f["path"] == "invoice.typ"));
}

#[tokio::test]
async fn preview_renders_a_sample() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());

    let (status, headers, bytes) = get_raw(&app, "/api/pdf/preview").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/pdf");
    let render = env.mock.last();
    assert_eq!(bytes, render.pdf);
    let data = render.data;
    assert_eq!(data["locale"], "cs", "the company default");
    assert_eq!(data["draft"], false);
    let year = chrono::Utc::now().format("%Y").to_string();
    assert_eq!(data["number"], format!("{year}0001"));
    assert_eq!(data["hasDiscount"], true);
    assert_eq!(data["qr"], Value::Null, "no default CZK account");
    let kinds: Vec<&str> = data["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .filter_map(|l| l["kind"].as_str())
        .collect();
    assert_eq!(kinds, ["item", "item", "item", "subtotal", "text"]);
    assert!(
        !env.storage.path().join(db.key("documents")).exists(),
        "never stored"
    );

    let (status, _, _) = get_raw(&app, "/api/pdf/preview?locale=en").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.mock.last().data["locale"], "en");

    let (status, err) = call(&app, Method::GET, "/api/pdf/preview?locale=de", None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": { "locale": "invalid" } })
    );
}

#[tokio::test]
async fn pdf_routes_need_the_token() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let id = uuid::Uuid::new_v4();
    for uri in [
        format!("/api/documents/{id}/pdf"),
        "/api/pdf/preview".to_string(),
        "/api/pdf/design".to_string(),
    ] {
        let resp = send(app.clone(), get(&uri, None)).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
    let (status, _) = call(&app, Method::GET, &format!("/api/documents/{id}/pdf"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_archive_file_is_500() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let (_, doc) = issue(&app, &id(&create_doc(&app, body).await)).await;
    let path = env
        .storage
        .path()
        .join(db.key(&common::documents::archive_rel(&db.conn, &doc).await));
    std::fs::remove_file(path).expect("remove archive");
    let (status, err) = call(
        &app,
        Method::GET,
        &format!("/api/documents/{}/pdf", id(&doc)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(err, json!({ "code": "internal" }));
}

#[tokio::test]
async fn race_loser_serves_the_stored_archive() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let (_, doc) = issue(&app, &id(&create_doc(&app, body).await)).await;
    let stored = env.mock.last().pdf;

    // As if this caller had seen no archive: it renders, loses the
    // `WHERE pdf_path IS NULL` update and must serve the winner's file.
    let pdf = common::space_pdf(&env.url, env.storage.storage.clone(), db.space);
    let doc_id = id(&doc).parse().expect("uuid");
    let bytes = invoice::pdf::archive::archive_missing(&db.conn, &pdf, doc_id)
        .await
        .expect("archive");
    assert_eq!(env.mock.count(), 2, "the loser did render");
    assert_ne!(env.mock.last().pdf, stored);
    assert_eq!(bytes, stored);
    assert_eq!(get_doc(&app, &id(&doc)).await["pdf"], doc["pdf"]);
}

#[tokio::test]
async fn cancelled_without_archive_renders_storno_without_qr() {
    use sea_orm::ConnectionTrait;
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    common::documents::set_company(&app, true).await;
    let contact = common::documents::create_contact(&app, json!({})).await;
    let (_, bank) = call(
        &app,
        Method::POST,
        "/api/settings/bank-accounts",
        Some(json!({ "currency": "CZK", "iban": "CZ6508000000192000145399" })),
    )
    .await;
    let body = json!({ "contactId": contact, "issueDate": "2026-10-01", "bankAccountId": bank["id"],
                       "lines": [item("1", "100", "21")] });
    let (_, doc) = issue(&app, &id(&create_doc(&app, body).await)).await;
    assert!(env.mock.last().data["qr"].is_object(), "issued with a QR");
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{}/cancel", id(&doc)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Like a document issued before archives existed.
    db.conn
        .execute_unprepared("UPDATE documents SET pdf_path = NULL, pdf_sha256 = NULL")
        .await
        .expect("drop archive");

    let (status, _, bytes) = get_raw(&app, &format!("/api/documents/{}/pdf", id(&doc))).await;
    assert_eq!(status, StatusCode::OK);
    let render = env.mock.last();
    assert_eq!(bytes, render.pdf);
    assert_eq!(render.data["watermark"], "STORNO");
    assert_eq!(render.data["draft"], false);
    assert_eq!(render.data["qr"], Value::Null);
    assert!(!render.assets.contains(&"qr.svg".to_string()));
    assert!(get_doc(&app, &id(&doc)).await["pdf"]["sha256"].is_string());
}
