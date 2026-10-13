//! PDF at issue: archive, failures roll the issue back, payload contents.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{
    archive_rel, create_contact, create_doc, dead_url, get_doc, id, issuable, issue, item,
    set_company,
};
use common::mdcast::{PdfEnv, get_raw};
use common::{TestDb, call};
use serde_json::{Value, json};

/// An issuable CZK draft paid by bank transfer to an account with an IBAN.
async fn qr_draft(app: &axum::Router) -> Value {
    set_company(app, true).await;
    let contact = create_contact(app, json!({})).await;
    let (status, bank) = call(
        app,
        Method::POST,
        "/api/settings/bank-accounts",
        Some(json!({ "currency": "CZK", "iban": "CZ6508000000192000145399", "bic": "GIBACZPX" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{bank}");
    json!({ "contactId": contact, "issueDate": "2026-10-01", "bankAccountId": bank["id"],
            "lines": [item("10", "100", "21")] })
}

#[tokio::test]
async fn issue_archives_the_pdf() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let draft = create_doc(&app, body).await;
    assert_eq!(draft["pdf"], Value::Null);

    let (status, doc) = issue(&app, &id(&draft)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let render = env.mock.last();
    let path = env
        .storage
        .path()
        .join(db.key(&archive_rel(&db.conn, &doc).await));
    let stored = std::fs::read(&path).expect("archived file exists");
    assert_eq!(stored, render.pdf);
    let sha = doc["pdf"]["sha256"].as_str().expect("sha256");
    assert_eq!(sha, invoiceapp::storage::sha256_hex(&stored));
    assert!(doc["pdf"]["renderedAt"].is_string());

    // What mdcast received: the issued document, fonts sorted, no template in the bundle.
    assert_eq!(render.data["draft"], false);
    assert_eq!(render.data["number"], "20260001");
    assert_eq!(render.data["title"], "Faktura – daňový doklad");
    assert_eq!(render.data["supplier"]["name"], "Dodavatel s.r.o.");
    assert_eq!(
        render.data["qr"],
        Value::Null,
        "the fixture account has no IBAN"
    );
    assert!(render.template.contains("/data.json"));
    assert!(!render.fonts.is_empty());
    let mut sorted = render.fonts.clone();
    sorted.sort();
    assert_eq!(render.fonts, sorted);
    assert!(render.fonts.iter().all(|f| render.assets.contains(f)));
    assert!(!render.assets.iter().any(|k| k == "invoice.typ"));
    assert!(env.mock.uploads() >= 1, "blob negotiation ran");

    // Downloads serve the archive without rendering again.
    let renders = env.mock.count();
    let uri = format!("/api/documents/{}/pdf", id(&doc));
    let (status, headers, bytes) = get_raw(&app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, stored);
    assert_eq!(headers["content-type"], "application/pdf");
    assert_eq!(
        headers["content-disposition"],
        "inline; filename=\"20260001.pdf\""
    );
    assert_eq!(headers["cache-control"], "no-store");
    let (_, headers, _) = get_raw(&app, &format!("{uri}?download=1")).await;
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"20260001.pdf\""
    );
    assert_eq!(env.mock.count(), renders);
}

#[tokio::test]
async fn mdcast_down_issues_nothing() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let down = env.router_at(db.conn.clone(), &dead_url());
    let body = issuable(&down).await;
    let draft = create_doc(&down, body).await;

    let (status, err) = issue(&down, &id(&draft)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{err}");
    assert_eq!(err, json!({ "code": "pdf_unavailable" }));
    let doc = get_doc(&down, &id(&draft)).await;
    assert_eq!(
        (&doc["status"], &doc["number"]),
        (&json!("draft"), &Value::Null)
    );
    assert_eq!(doc["supplier"], Value::Null);

    // The counter did not move: the retry gets the first number.
    let up = env.router(db.conn.clone());
    let (status, doc) = issue(&up, &id(&draft)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["number"], "20260001");
}

#[tokio::test]
async fn render_error_is_502_with_detail() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let draft = create_doc(&app, body).await;
    env.mock.fail_with("error: unknown variable: totalz");

    let (status, err) = issue(&app, &id(&draft)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        err,
        json!({ "code": "pdf_render_failed", "detail": "error: unknown variable: totalz" })
    );
    assert_eq!(get_doc(&app, &id(&draft)).await["status"], "draft");
    assert!(
        !env.storage.path().join(db.key("documents")).exists(),
        "nothing archived"
    );
}

#[tokio::test]
async fn upstream_failure_is_502_without_detail() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let draft = create_doc(&app, body).await;
    env.mock
        .fail_as(500, "internal", "panic at /srv/mdcast/src/render.rs:42");

    let (status, err) = issue(&app, &id(&draft)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(err, json!({ "code": "pdf_render_failed" }));
    assert_eq!(get_doc(&app, &id(&draft)).await["status"], "draft");
}

#[tokio::test]
async fn qr_only_for_bank_transfer_with_iban() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = qr_draft(&app).await;

    let (status, doc) = issue(&app, &id(&create_doc(&app, body.clone()).await)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let render = env.mock.last();
    assert_eq!(
        render.data["qr"],
        json!({ "image": "qr.svg", "label": "QR platba" })
    );
    assert!(render.assets.contains(&"qr.svg".to_string()));

    let mut cash = body.clone();
    cash["paymentMethod"] = json!("cash");
    let (status, _) = issue(&app, &id(&create_doc(&app, cash).await)).await;
    assert_eq!(status, StatusCode::OK);
    let render = env.mock.last();
    assert_eq!(render.data["qr"], Value::Null);
    assert!(!render.assets.contains(&"qr.svg".to_string()));

    // A draft never carries a QR, and is never stored.
    let draft = create_doc(&app, body).await;
    let (status, headers, bytes) =
        get_raw(&app, &format!("/api/documents/{}/pdf", id(&draft))).await;
    assert_eq!(status, StatusCode::OK);
    let render = env.mock.last();
    assert_eq!(bytes, render.pdf);
    assert_eq!(render.data["draft"], true);
    assert_eq!(render.data["watermark"], "NÁVRH");
    assert_eq!(render.data["qr"], Value::Null);
    assert_eq!(render.data["number"], Value::Null);
    let short = &id(&draft).replace('-', "")[..8];
    assert_eq!(
        headers["content-disposition"],
        format!("inline; filename=\"draft-{short}.pdf\"").as_str()
    );
    assert_eq!(get_doc(&app, &id(&draft)).await["pdf"], Value::Null);
    let year_dir = env.storage.path().join(db.key("documents/2026"));
    let files = std::fs::read_dir(year_dir).expect("archive dir").count();
    assert_eq!(files, 2, "only the two issued documents are archived");
}

#[tokio::test]
async fn credit_note_prints_negated() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let (_, invoice) = issue(&app, &id(&create_doc(&app, body).await)).await;
    let (status, cn) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{}/credit-note", id(&invoice)),
        Some(json!({ "correctionReason": "Reklamace" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{cn}");
    let (status, cn) = issue(&app, &id(&cn)).await;
    assert_eq!(status, StatusCode::OK, "{cn}");

    let data = env.mock.last().data;
    assert_eq!(data["docType"], "credit_note");
    assert_eq!(data["title"], "Opravný daňový doklad");
    assert_eq!(
        data["reference"],
        "Opravný daňový doklad k faktuře 20260001\nDůvod opravy: Reklamace"
    );
    assert_eq!(data["lines"][0]["base"], "-1\u{a0}000,00\u{a0}Kč");
    assert_eq!(data["lines"][0]["unitPrice"], "-100,00\u{a0}Kč");
    let totals = data["totals"].as_array().expect("totals");
    assert_eq!(
        totals.last().map(|t| &t["value"]),
        Some(&json!("-1\u{a0}210,00\u{a0}Kč"))
    );
    assert_eq!(data["qr"], Value::Null);
}
