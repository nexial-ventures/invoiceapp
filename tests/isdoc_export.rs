//! ISDOC export: single `.isdocx` / `.isdoc`, bulk ZIP, XSD validity, and the
//! round trip import(export(doc)).

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{
    create_bank, create_contact, create_doc, create_issued, get_doc, id, issuable, issue, item,
    set_company,
};
use common::isdoc::{assert_valid, confirm, unzip};
use common::mdcast::{PdfEnv, get_raw};
use common::received::{create_received, upload};
use common::{TestDb, call};
use serde_json::{Value, json};

fn text(description: &str) -> Value {
    json!({ "kind": "text", "description": description })
}

fn entry<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> &'a [u8] {
    &files
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| {
            panic!(
                "no {name} in {:?}",
                files.iter().map(|f| &f.0).collect::<Vec<_>>()
            )
        })
        .1
}

#[tokio::test]
async fn native_invoice_exports_as_a_valid_isdocx() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let mut body = issuable(&app).await;
    body["lines"] = json!([
        item("10", "100", "21"),
        text("Poznámka"),
        item("2", "50", "12")
    ]);
    body["headerNote"] = json!("Děkujeme & na shledanou");
    let doc = create_issued(&app, body).await;
    let doc_id = id(&doc);
    let number = doc["number"].as_str().expect("number").to_string();

    let (status, headers, bytes) = get_raw(&app, &format!("/api/documents/{doc_id}/isdoc")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/zip");
    assert_eq!(
        headers["content-disposition"],
        format!("attachment; filename=\"{number}.isdocx\"").as_str()
    );
    let files = unzip(&bytes);
    let xml = entry(&files, &format!("{number}.isdoc"));
    assert_valid(xml, "isdoc-invoice-6.0.2.xsd");
    assert_valid(entry(&files, "manifest.xml"), "isdoc-manifest-6.0.2.xsd");
    let (_, _, archived) = get_raw(&app, &format!("/api/documents/{doc_id}/pdf")).await;
    assert_eq!(entry(&files, &format!("{number}.pdf")), archived.as_slice());
    let xml = String::from_utf8(xml.to_vec()).expect("utf-8");
    assert!(xml.contains(&format!("<Filename>{number}.pdf</Filename>")));
    assert!(xml.contains("<VATApplicable>true</VATApplicable>"));
    assert!(xml.contains("<PayableAmount>1322.00</PayableAmount>"));

    // Credit note: positive amounts and the original's reference.
    let (status, draft) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{doc_id}/credit-note"),
        Some(json!({ "correctionReason": "Reklamace" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    let (status, credit) = issue(&app, &id(&draft)).await;
    assert_eq!(status, StatusCode::OK, "{credit}");
    let (_, _, bytes) = get_raw(&app, &format!("/api/documents/{}/isdoc", id(&credit))).await;
    let files = unzip(&bytes);
    let cn = credit["number"].as_str().expect("number");
    let xml = entry(&files, &format!("{cn}.isdoc"));
    assert_valid(xml, "isdoc-invoice-6.0.2.xsd");
    let xml = String::from_utf8(xml.to_vec()).expect("utf-8");
    assert!(xml.contains("<DocumentType>2</DocumentType>"));
    assert!(xml.contains(&format!("<OriginalDocumentReference><ID>{number}</ID>")));
    assert!(xml.contains("Důvod opravy: Reklamace"));
    assert!(!xml.contains(">-"), "credit notes are exported positive");
}

#[tokio::test]
async fn foreign_currency_export_is_valid() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    set_company(&app, true).await;
    let contact = create_contact(&app, json!({})).await;
    create_bank(&app, "EUR").await;
    let doc = create_issued(
        &app,
        json!({ "contactId": contact, "currency": "EUR", "exchangeRate": "24.5",
                "issueDate": "2026-10-01", "lines": [item("3", "33.3333", "21")] }),
    )
    .await;
    let (_, _, bytes) = get_raw(&app, &format!("/api/documents/{}/isdoc", id(&doc))).await;
    let files = unzip(&bytes);
    let xml = entry(
        &files,
        &format!("{}.isdoc", doc["number"].as_str().expect("n")),
    );
    assert_valid(xml, "isdoc-invoice-6.0.2.xsd");
    let xml = String::from_utf8(xml.to_vec()).expect("utf-8");
    assert!(
        xml.contains("<ForeignCurrencyCode>EUR</ForeignCurrencyCode><CurrRate>24.5</CurrRate>")
    );
}

#[tokio::test]
async fn states_and_plain_isdoc_without_pdf() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let base = issuable(&app).await;
    let draft = create_doc(&app, base.clone()).await;
    let (status, body) = call(
        &app,
        Method::GET,
        &format!("/api/documents/{}/isdoc", id(&draft)),
        None,
    )
    .await;
    assert_eq!(
        (status, &body["code"]),
        (StatusCode::CONFLICT, &json!("invalid_state"))
    );
    let supplier = create_contact(&app, json!({ "name": "Dodavatel X", "ico": "87654326" })).await;
    let received = create_received(&app, &supplier, json!({})).await;
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/documents/{}/isdoc", id(&received)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Imported without an original: a plain `.isdoc`; with one: an `.isdocx`.
    let mut imported = base.clone();
    imported["imported"] = json!(true);
    imported["number"] = json!("FV-2019/1");
    let imp = create_doc(&app, imported).await;
    let (status, doc) = issue(&app, &id(&imp)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let (status, headers, bytes) =
        get_raw(&app, &format!("/api/documents/{}/isdoc", id(&imp))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/xml");
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"FV-2019_1.isdoc\""
    );
    assert_valid(&bytes, "isdoc-invoice-6.0.2.xsd");
    let pdf = b"%PDF-1.4\n% original\n";
    let (status, _) = upload(&app, &id(&imp), pdf).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, headers, bytes) = get_raw(&app, &format!("/api/documents/{}/isdoc", id(&imp))).await;
    assert_eq!(headers["content-type"], "application/zip");
    assert_eq!(entry(&unzip(&bytes), "FV-2019_1.pdf"), pdf.as_slice());
    assert_eq!(env.mock.count(), 0, "imported documents are never rendered");

    // A native document whose PDF cannot be rendered → plain `.isdoc`.
    let issued = create_issued(&app, base).await;
    let down = env.router_at(db.conn.clone(), &common::documents::dead_url());
    // Drop the archive so the export would have to render it.
    let row = invoiceapp::document::entity::document::Entity::find_by_id(
        id(&issued).parse::<uuid::Uuid>().expect("uuid"),
    );
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let model = row.one(&db.conn).await.expect("query").expect("row");
    let mut active: invoiceapp::document::entity::document::ActiveModel = model.into();
    active.pdf_path = Set(None);
    active.update(&db.conn).await.expect("update");
    let (status, headers, _) =
        get_raw(&down, &format!("/api/documents/{}/isdoc", id(&issued))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/xml");
}

#[tokio::test]
async fn bulk_export_follows_the_list_filters() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let base = issuable(&app).await;
    let a = create_issued(&app, base.clone()).await;
    let b = create_issued(&app, base.clone()).await;
    create_doc(&app, base).await;
    let (status, headers, bytes) = get_raw(&app, "/api/documents/isdoc").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"isdoc-export.zip\""
    );
    let mut names: Vec<String> = unzip(&bytes).into_iter().map(|f| f.0).collect();
    names.sort();
    let mut want = [a["number"].clone(), b["number"].clone()]
        .map(|n| format!("{}.isdocx", n.as_str().unwrap_or("")));
    want.sort();
    assert_eq!(names, want, "drafts are excluded");
    let (_, _, bytes) = get_raw(
        &app,
        &format!(
            "/api/documents/isdoc?q={}",
            a["number"].as_str().unwrap_or("")
        ),
    )
    .await;
    assert_eq!(unzip(&bytes).len(), 1);
    let (_, _, bytes) = get_raw(&app, "/api/documents/isdoc?status=draft").await;
    assert_eq!(unzip(&bytes).len(), 0);
}

#[tokio::test]
async fn round_trip_reproduces_the_document() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let mut body = issuable(&app).await;
    body["lines"] = json!([
        item("10", "100", "21"),
        text("Dodáno"),
        item("3", "33.3333", "12")
    ]);
    let doc = create_issued(&app, body).await;
    let (_, _, isdocx) = get_raw(&app, &format!("/api/documents/{}/isdoc", id(&doc))).await;

    let db2 = TestDb::new().await;
    let other = env.router(db2.conn.clone());
    set_company(&other, true).await;
    let files = [("export.isdocx", isdocx.as_slice())];
    let results = confirm(
        &other,
        &files,
        json!({ "selected": ["export.isdocx"], "markPaid": false }),
    )
    .await;
    assert_eq!(results[0]["status"], "imported", "{results:?}");
    let back = get_doc(&other, results[0]["documentId"].as_str().expect("id")).await;
    for field in [
        "number",
        "issueDate",
        "taxPointDate",
        "dueDate",
        "currency",
        "vatMode",
        "docType",
    ] {
        assert_eq!(back[field], doc[field], "{field}");
    }
    assert_eq!(back["totals"], doc["totals"]);
    assert_eq!(back["customer"]["ico"], doc["customer"]["ico"]);
    assert_eq!(back["supplier"]["ico"], doc["supplier"]["ico"]);
    let strip = |lines: &Value| -> Vec<Value> {
        lines
            .as_array()
            .expect("lines")
            .iter()
            .map(|l| {
                json!([
                    l["kind"],
                    l["description"],
                    l["quantity"],
                    l["unitPrice"],
                    l["vatRate"],
                    l["base"]
                ])
            })
            .collect()
    };
    assert_eq!(strip(&back["lines"]), strip(&doc["lines"]));
    assert_eq!(back["imported"], true);
    let (_, _, pdf) = get_raw(&other, &format!("/api/documents/{}/pdf", id(&back))).await;
    let (_, _, original) = get_raw(&app, &format!("/api/documents/{}/pdf", id(&doc))).await;
    assert_eq!(pdf, original, "the archived PDF became the original");

    // The other direction: a third database where the customer is the
    // company receives the same export with its lines.
    let db3 = TestDb::new().await;
    let receiver = env.router(db3.conn.clone());
    set_company(&receiver, true).await;
    let (status, _) = call(
        &receiver,
        Method::PUT,
        "/api/settings/company",
        Some(
            json!({ "name": "Odběratel a.s.", "ico": "12345679", "vatPayer": true,
                     "street": "", "city": "Brno", "zip": "", "country": "CZ", "defaultDueDays": 14,
                     "defaultLocale": "cs" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let results = confirm(&receiver, &files, json!({ "selected": ["export.isdocx"] })).await;
    let got = get_doc(&receiver, results[0]["documentId"].as_str().expect("id")).await;
    assert_eq!(got["direction"], "received");
    assert_eq!(got["supplierNumber"], doc["number"]);
    assert_eq!(got["totals"]["recap"], doc["totals"]["recap"]);
    assert_eq!(strip(&got["lines"]), strip(&doc["lines"]));
}

#[tokio::test]
async fn bulk_export_skips_a_failing_document() {
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let base = issuable(&app).await;
    let good = create_issued(&app, base.clone()).await;
    let bad = create_issued(&app, base).await;
    // A broken row (no supplier snapshot) cannot be exported.
    let row = invoiceapp::document::entity::document::Entity::find_by_id(
        id(&bad).parse::<uuid::Uuid>().expect("uuid"),
    )
    .one(&db.conn)
    .await
    .expect("query")
    .expect("row");
    let mut active: invoiceapp::document::entity::document::ActiveModel = row.into();
    active.supplier_snapshot = Set(None);
    active.update(&db.conn).await.expect("update");

    let (status, _, bytes) = get_raw(&app, "/api/documents/isdoc").await;
    assert_eq!(status, StatusCode::OK);
    let files = unzip(&bytes);
    let good_name = format!("{}.isdocx", good["number"].as_str().expect("n"));
    assert!(files.iter().any(|f| f.0 == good_name));
    let errors = String::from_utf8(entry(&files, "errors.txt").to_vec()).expect("utf-8");
    let bad_number = bad["number"].as_str().expect("n");
    assert_eq!(errors, format!("{bad_number}: export failed (internal)\n"));
    assert_eq!(files.len(), 2);
    // Nothing exportable at all → the error itself.
    let (status, body) = call(
        &app,
        Method::GET,
        &format!("/api/documents/isdoc?q={bad_number}"),
        None,
    )
    .await;
    assert_eq!(
        (status, &body["code"]),
        (StatusCode::INTERNAL_SERVER_ERROR, &json!("internal"))
    );
}
