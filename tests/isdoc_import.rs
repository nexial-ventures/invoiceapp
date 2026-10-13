//! ISDOC bulk import: preview, confirm (mark paid, duplicates, related links
//! within a batch, contacts), originals from `.isdocx`, exchange rates.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{create_bank, create_contact, dead_url, get_doc, mock_cnb, set_company};
use common::isdoc::{confirm, fixture, form, post_form, preview, zip_of};
use common::mdcast::{PdfEnv, get_raw};
use common::received::{create_category, create_received, received_body};
use common::{TestDb, call, router, router_with_cnb};
use serde_json::{Value, json};

fn by_key<'a>(rows: &'a [Value], key: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["key"] == key)
        .unwrap_or_else(|| panic!("no entry {key} in {rows:?}"))
}

fn batch() -> Vec<u8> {
    let inner = zip_of(&[
        (
            "received_vat.isdoc",
            fixture("received_vat.isdoc").as_bytes(),
        ),
        (
            "received_credit.isdoc",
            fixture("received_credit.isdoc").as_bytes(),
        ),
    ]);
    zip_of(&[
        (
            "issued_nonpayer.isdoc",
            fixture("issued_nonpayer.isdoc").as_bytes(),
        ),
        ("2026.zip", &inner),
        ("readme.txt", b"ignored"),
    ])
}

const ISSUED: &str = "invoices.zip/issued_nonpayer.isdoc";
const RECEIVED: &str = "invoices.zip/2026.zip/received_vat.isdoc";
const CREDIT: &str = "invoices.zip/2026.zip/received_credit.isdoc";

#[tokio::test]
async fn preview_reports_every_entry() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, false).await;
    let customer = create_contact(&app, json!({ "name": "Odběratel", "ico": "12345679" })).await;
    assert!(!customer.is_empty());
    let foreign = fixture("received_vat.isdoc").replace("44444443", "11111111");
    let zip = batch();
    let rows = preview(
        &app,
        &[
            ("invoices.zip", &zip),
            ("bad.isdoc", b"<not-isdoc/>"),
            ("foreign.isdoc", foreign.as_bytes()),
        ],
    )
    .await;
    let keys: Vec<&str> = rows
        .iter()
        .map(|r| r["key"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        keys,
        [ISSUED, RECEIVED, CREDIT, "bad.isdoc", "foreign.isdoc"]
    );

    let i = by_key(&rows, ISSUED);
    assert_eq!(i["status"], "ok");
    assert_eq!(i["direction"], "issued");
    assert_eq!(i["docType"], "invoice");
    assert_eq!(i["number"], "202600011");
    assert_eq!(
        i["counterparty"],
        json!({ "name": "Fiktivní Odběratel s.r.o.", "ico": "12345679" })
    );
    assert_eq!(i["contactMatch"], "existing");
    assert_eq!(i["taxPointDate"], "2026-10-01");
    assert_eq!(i["dueDate"], "2026-10-16");
    assert_eq!(i["total"], "24000.00");
    assert_eq!(i["hasPdf"], false);
    assert_eq!(i["warnings"], json!([]), "PaidAmount is ignored silently");

    let r = by_key(&rows, RECEIVED);
    assert_eq!(r["direction"], "received");
    assert_eq!(r["contactMatch"], "new");
    assert_eq!(r["warnings"], json!(["contact_created"]));
    let c = by_key(&rows, CREDIT);
    assert_eq!(c["docType"], "credit_note");
    assert_eq!(c["relatedNumber"], "FV-2026/077");
    assert_eq!(c["relatedFound"], true, "the original is in the batch");

    assert_eq!(by_key(&rows, "bad.isdoc")["status"], "error");
    assert_eq!(by_key(&rows, "bad.isdoc")["error"], "invalid_xml");
    assert_eq!(by_key(&rows, "foreign.isdoc")["error"], "foreign");
}

#[tokio::test]
async fn confirm_imports_marks_paid_links_and_skips_duplicates() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, false).await;
    let bank = create_bank(&app, "CZK").await;
    let category = create_category(&app, "Software", "expense").await;
    let zip = batch();
    let files = [("invoices.zip", zip.as_slice())];
    let results = confirm(
        &app,
        &files,
        json!({ "selected": [CREDIT, ISSUED, RECEIVED], "categoryId": category, "vatDeductible": false }),
    )
    .await;
    for key in [ISSUED, RECEIVED, CREDIT] {
        assert_eq!(by_key(&results, key)["status"], "imported", "{results:?}");
    }
    let doc_id = |key: &str| {
        by_key(&results, key)["documentId"]
            .as_str()
            .expect("id")
            .to_string()
    };

    let issued = get_doc(&app, &doc_id(ISSUED)).await;
    assert_eq!(
        (&issued["imported"], &issued["status"]),
        (&json!(true), &json!("issued"))
    );
    assert_eq!(issued["number"], "202600011");
    assert_eq!(issued["vatMode"], "non_payer");
    assert_eq!(issued["paid"], "24000.00");
    assert_eq!(issued["paymentState"], "paid");
    assert_eq!(issued["lines"].as_array().map(Vec::len), Some(2));
    assert_eq!(issued["customer"]["ico"], "12345679");
    assert_eq!(issued["supplier"]["vatPayer"], false);
    assert_eq!(issued["bankAccountId"], json!(bank));
    assert_eq!(issued["totals"]["payable"], "24000.00");
    assert_eq!(issued["locale"], "cs");

    let received = get_doc(&app, &doc_id(RECEIVED)).await;
    assert_eq!(received["number"], "P20260001");
    assert_eq!(received["supplierNumber"], "FV-2026/077");
    assert_eq!(received["receivedDate"], "2026-09-08");
    assert_eq!(received["categoryId"], json!(category));
    assert_eq!(received["vatDeductible"], false);
    assert_eq!(received["supplier"]["ico"], "87654326");
    assert_eq!(received["internalNote"], "Děkujeme za objednávku");
    assert_eq!(received["supplierAccount"], "CZ0001000000000123456789");
    assert_eq!(
        received["totals"]["recap"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(received["paid"], "1322.00");
    // Lines are stored for received documents too (display only).
    let kinds = |d: &Value| -> Vec<Value> {
        d["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .map(|l| l["kind"].clone())
            .collect()
    };
    assert_eq!(
        kinds(&received),
        [json!("item"), json!("item"), json!("text")]
    );
    assert_eq!(received["lines"][0]["base"], "1000.00");
    // A received PUT carries no lines and keeps them; the recap is the PUT's.
    let contact = received["contactId"].as_str().expect("contact").to_string();
    let (status, put) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{}", doc_id(RECEIVED)),
        Some(received_body(
            &contact,
            json!({ "supplierNumber": "FV-2026/077", "currency": "CZK", "vatMode": "standard" }),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{put}");
    assert_eq!(kinds(&put), [json!("item"), json!("item"), json!("text")]);
    assert_eq!(put["totals"]["recap"].as_array().map(Vec::len), Some(1));
    let native = create_received(&app, &contact, json!({ "supplierNumber": "X-1" })).await;
    assert_eq!(native["lines"], json!([]));

    let credit = get_doc(&app, &doc_id(CREDIT)).await;
    assert_eq!(credit["relatedDocumentId"], json!(doc_id(RECEIVED)));
    assert_eq!(credit["paid"], "1322.00", "refund recorded");
    assert_eq!(credit["number"], "PD20260001");

    // One supplier contact created, reused by the credit note; the
    // customer of the issued invoice is new too.
    let (_, contacts) = call(&app, Method::GET, "/api/contacts", None).await;
    let names: Vec<&str> = contacts["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(credit["contactId"], received["contactId"]);

    let again = preview(&app, &files).await;
    for key in [ISSUED, RECEIVED, CREDIT] {
        assert_eq!(by_key(&again, key)["status"], "duplicate");
    }
    let results = confirm(
        &app,
        &files,
        json!({ "selected": [ISSUED, RECEIVED, CREDIT] }),
    )
    .await;
    for key in [ISSUED, RECEIVED, CREDIT] {
        assert_eq!(by_key(&results, key)["status"], "skipped");
    }
}

#[tokio::test]
async fn confirm_options() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, false).await;
    let zip = batch();
    let files = [("invoices.zip", zip.as_slice())];
    for options in [None, Some("{not json")] {
        let (status, body) =
            post_form(&app, "/api/import/isdoc/confirm", form(&files, options)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["fields"]["options"], "invalid");
    }
    let income = create_category(&app, "Tržby", "income").await;
    let o = json!({ "selected": [], "categoryId": income }).to_string();
    let (status, body) = post_form(&app, "/api/import/isdoc/confirm", form(&files, Some(&o))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["categoryId"], "invalid");
    let (status, body) = post_form(&app, "/api/import/isdoc/preview", form(&[], None)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["files"], "required");

    // Only the issued one, not paid; the credit note alone finds no original.
    let results = confirm(
        &app,
        &files,
        json!({ "selected": [ISSUED, CREDIT], "markPaid": false }),
    )
    .await;
    assert_eq!(by_key(&results, RECEIVED)["status"], "skipped");
    let issued = get_doc(
        &app,
        by_key(&results, ISSUED)["documentId"].as_str().expect("id"),
    )
    .await;
    assert_eq!(issued["paid"], "0.00");
    let credit = get_doc(
        &app,
        by_key(&results, CREDIT)["documentId"].as_str().expect("id"),
    )
    .await;
    assert_eq!(credit["relatedDocumentId"], Value::Null);
    // Now the credit note is a duplicate and its original is still missing.
    let rows = preview(&app, &files).await;
    assert_eq!(by_key(&rows, CREDIT)["status"], "duplicate");
}

#[tokio::test]
async fn isdocx_original_pdf_is_stored() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    set_company(&app, true).await;
    let pdf = b"%PDF-1.4\n% isdocx test original\n%%EOF\n";
    let manifest = br#"<?xml version="1.0"?><manifest xmlns="http://isdoc.cz/namespace/2013/manifest"><maindocument filename="doc.isdoc"/></manifest>"#;
    let isdocx = zip_of(&[
        ("manifest.xml", manifest),
        ("doc.isdoc", fixture("received_vat.isdoc").as_bytes()),
        ("doc.pdf", pdf),
    ]);
    let files = [("doc.isdocx", isdocx.as_slice())];
    let rows = preview(&app, &files).await;
    assert_eq!(rows[0]["hasPdf"], true);
    let results = confirm(&app, &files, json!({ "selected": ["doc.isdocx"] })).await;
    let id = results[0]["documentId"].as_str().expect("id").to_string();
    let (status, _, bytes) = get_raw(&app, &format!("/api/documents/{id}/pdf")).await;
    assert_eq!((status, bytes.as_slice()), (StatusCode::OK, pdf.as_slice()));
    assert_eq!(env.mock.count(), 0, "never rendered");
}

#[tokio::test]
async fn foreign_currency_rates() {
    let db = TestDb::new().await;
    let (cnb, _) = mock_cnb().await;
    let app = router_with_cnb(db.conn.clone(), &cnb);
    set_company(&app, true).await;
    let with_rate = fixture("received_eur.isdoc");
    let without = with_rate
        .replace("<CurrRate>24.335</CurrRate>", "<CurrRate>1</CurrRate>")
        .replace("INV-EUR-5", "INV-EUR-6");
    let files = [
        ("a.isdoc", with_rate.as_bytes()),
        ("b.isdoc", without.as_bytes()),
    ];
    let rows = preview(&app, &files).await;
    assert_eq!(
        rows[1]["warnings"],
        json!(["rate_from_cnb", "contact_created"])
    );
    let results = confirm(&app, &files, json!({ "selected": ["a.isdoc", "b.isdoc"] })).await;
    let a = get_doc(&app, results[0]["documentId"].as_str().expect("id")).await;
    assert_eq!(
        (&a["currency"], &a["exchangeRate"]),
        (&json!("EUR"), &json!("24.335"))
    );
    assert_eq!(a["exchangeRateSource"], "manual");
    assert_eq!(a["vatMode"], "reverse_charge");
    assert_eq!(a["totals"]["totalCzk"], "24335.00");
    let b = get_doc(&app, results[1]["documentId"].as_str().expect("id")).await;
    assert_eq!(
        (&b["exchangeRate"], &b["exchangeRateSource"]),
        (&json!("25.125"), &json!("cnb"))
    );

    // ČNB down at confirm: that entry fails, the others still import.
    let db2 = TestDb::new().await;
    let down = router_with_cnb(db2.conn.clone(), &dead_url());
    set_company(&down, true).await;
    let results = confirm(&down, &files, json!({ "selected": ["a.isdoc", "b.isdoc"] })).await;
    assert_eq!(results[0]["status"], "imported");
    assert_eq!(results[1]["status"], "failed");
    assert_eq!(results[1]["error"], "rate_unavailable");
}

#[tokio::test]
async fn confirm_rechecks_duplicates_under_a_lock() {
    use invoiceapp::document::repo::issue::Rate;
    use invoiceapp::error::AppError;
    use invoiceapp::import::store::{self, Options};
    use invoiceapp::isdoc::{parse, plan};

    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let pdf = common::space_pdf(&dead_url(), common::shared_storage(), db.space);
    let opts = Options {
        paid_on: None,
        category: None,
        vat_deductible: true,
    };
    let rate = Rate {
        rate: None,
        date: None,
        source: None,
    };
    let received = |name: &str| {
        let parsed = parse::parse(fixture(name).as_bytes()).expect("parsed");
        plan::plan(parsed, Some("44444443")).expect("plan")
    };
    // Sequential: the second import of the same received document (no
    // unique index backs it) is refused inside its transaction.
    let p = received("received_vat.isdoc");
    store::import(&db.conn, &pdf, &p, None, &rate, &opts)
        .await
        .expect("first import");
    match store::import(&db.conn, &pdf, &p, None, &rate, &opts).await {
        Err(AppError::Duplicate) => {}
        other => panic!("expected a duplicate, got {other:?}"),
    }
    // Concurrent: the advisory lock lets exactly one through.
    let c = received("received_credit.isdoc");
    let (a, b) = tokio::join!(
        store::import(&db.conn, &pdf, &c, None, &rate, &opts),
        store::import(&db.conn, &pdf, &c, None, &rate, &opts),
    );
    assert_eq!(
        usize::from(a.is_ok()) + usize::from(b.is_ok()),
        1,
        "{a:?} {b:?}"
    );
    let (_, list) = call(&app, Method::GET, "/api/documents?direction=received", None).await;
    assert_eq!(list["total"], 2);
}

#[tokio::test]
async fn foreign_local_currency_is_unsupported() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let eur = fixture("received_vat.isdoc").replace(
        "<LocalCurrencyCode>CZK</LocalCurrencyCode>",
        "<LocalCurrencyCode>EUR</LocalCurrencyCode>",
    );
    let rows = preview(&app, &[("sk.isdoc", eur.as_bytes())]).await;
    assert_eq!(
        (&rows[0]["status"], &rows[0]["error"]),
        (&json!("error"), &json!("unsupported_currency"))
    );
}
