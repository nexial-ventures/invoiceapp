//! Pohoda XML (`GET /api/export/accountant?format=pohoda`): every exported
//! type in both directions validated against the Stormware XSD, settings
//! codes, the third rate slot, skipped documents, format errors.

mod common;

use axum::http::{Method, StatusCode};
use common::csv_export::{export_error, get_raw, import_all};
use common::documents::{create_bank, create_contact, create_issued, item, set_company};
use common::pohoda::{JANUARY, export, fixture, item as find, items};
use common::{TestDb, call, router};
use serde_json::json;

async fn add_rate_10(app: &axum::Router) {
    let (status, body) = call(
        app,
        Method::POST,
        "/api/settings/vat-rates",
        Some(json!({ "rate": "10", "label": "Druhá snížená", "isDefault": false, "active": true })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

fn has(item: &str, parts: &[&str]) {
    for p in parts {
        assert!(item.contains(p), "{p}\nin\n{item}");
    }
}

#[tokio::test]
async fn every_type_in_both_directions() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    create_bank(&app, "CZK").await;
    import_all(&app, &fixture()).await;
    let contact = create_contact(&app, json!({ "name": "Nativní Odběratel", "ico": null })).await;
    create_issued(
        &app,
        json!({ "contactId": contact, "issueDate": "2026-01-20", "taxPointDate": "2026-01-20",
                "lines": [item("1", "5", "21")] }),
    )
    .await;

    // FV-50 at 10 % has no Pohoda slot: no file, the accountant is told.
    let (status, body) = export_error(&app, JANUARY).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({ "code": "validation", "fields": { "documents": "unexportable" },
                "detail": "FV-50: VAT rate 10 % maps to no Pohoda rate slot" })
    );
    add_rate_10(&app).await;

    let (status, headers, bytes) = get_raw(&app, JANUARY).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"pohoda-2026-01-01-2026-01-31.xml\""
    );
    assert!(bytes.starts_with(b"<?xml version=\"1.0\" encoding=\"Windows-1250\"?>"));
    assert!(
        bytes.windows(7).any(|w| w == b"&#1059;"),
        "a character outside Windows-1250 is a character reference"
    );
    let xml = export(&app, JANUARY).await;
    has(
        &xml,
        &[
            "ico=\"44444443\" application=\"InvoiceApp\" version=\"2.0\"",
            "note=\"Export z Invoice 2026-01-01–2026-01-31\"",
        ],
    );
    // 17 documents − the proforma = 16.
    assert_eq!(items(&xml).len(), 16, "{xml}");
    assert!(!xml.contains(">ZF-1</typ:numberRequested>"), "{xml}");

    let invoice = |t| format!("<inv:invoiceType>{t}</inv:invoiceType>");
    has(
        find(&xml, "Faktura FA-1"),
        &[
            &invoice("issuedInvoice"),
            "<typ:numberRequested>FA-1</typ:numberRequested>",
            "<inv:symVar>1</inv:symVar><inv:originalDocument>FA-1</inv:originalDocument>",
            "<inv:dateTax>2026-01-15</inv:dateTax><inv:dateAccounting>2026-01-15</inv:dateAccounting>\
             <inv:dateDue>2026-01-29</inv:dateDue>",
            "<typ:company>Fiktivní Odběratel s.r.o.</typ:company><typ:city>Brno</typ:city>\
             <typ:street>Zkušební 12</typ:street><typ:zip>60200</typ:zip><typ:ico>12345679</typ:ico>",
            "<typ:priceHigh>1000.00</typ:priceHigh><typ:priceHighVAT>210.00</typ:priceHighVAT>",
        ],
    );
    has(
        find(&xml, "Dobropis DB-1 k FA-1"),
        &[
            &invoice("issuedCreditNotice"),
            "<typ:priceHigh>-100.00</typ:priceHigh><typ:priceHighVAT>-21.00</typ:priceHighVAT>",
        ],
    );
    has(
        find(&xml, "Vrubopis VB-1 k FA-1"),
        &[&invoice("issuedDebitNote"), "<typ:priceHighVAT>10.50<"],
    );
    has(
        find(&xml, "Daňový doklad k platbě DD-1 k ZF-1"),
        &[
            "<int:intDoc version=\"2.0\">",
            "<typ:priceHigh>500.00</typ:priceHigh>",
        ],
    );
    has(
        find(&xml, "Opravný daňový doklad k platbě OD-1 k DD-1"),
        &["<int:intDoc ", "<typ:priceHigh>-100.00</typ:priceHigh>"],
    );
    let simplified = find(&xml, "Zjednodušený daňový doklad ZJ-1");
    has(
        simplified,
        &[
            &invoice("issuedInvoice"),
            "<typ:priceLow>100.00</typ:priceLow>",
            "<typ:priceRound>-0.40<",
        ],
    );
    assert!(!simplified.contains("partnerIdentity"), "contactless");
    has(
        find(&xml, "Faktura FA-3"),
        &[
            "<typ:city>München</typ:city>",
            "<typ:country><typ:ids>DE</typ:ids></typ:country>",
            "<typ:priceHigh>2500.00</typ:priceHigh>",
            "<inv:foreignCurrency><typ:currency><typ:ids>EUR</typ:ids></typ:currency><typ:rate>25</typ:rate>\
             <typ:amount>1</typ:amount><typ:priceSum>121.00</typ:priceSum></inv:foreignCurrency>",
        ],
    );
    let native = items(&xml)
        .into_iter()
        .find(|i| i.contains("Nativní Odběratel"))
        .expect("native invoice");
    has(
        native,
        &[
            "<inv:account><typ:accountNo>19-2000145399</typ:accountNo><typ:bankCode>0800</typ:bankCode></inv:account>",
        ],
    );

    has(
        find(&xml, "Přijatá faktura FV-42"),
        &[
            &invoice("receivedInvoice"),
            "<inv:originalDocument>FV-42</inv:originalDocument>",
            "<inv:dateAccounting>2026-01-22</inv:dateAccounting>",
            "<typ:priceHigh>2433.50</typ:priceHigh><typ:priceHighVAT>511.04</typ:priceHighVAT>",
            "<typ:rate>24.335</typ:rate>",
            "<typ:classificationVATType>nonSubsume</typ:classificationVATType>",
        ],
    );
    has(
        find(&xml, "Přijatý dobropis FV-45 k FV-44"),
        &[&invoice("receivedCreditNotice"), "<typ:priceHigh>-200.00<"],
    );
    has(
        find(&xml, "Přijatý vrubopis FV-46 k FV-44"),
        &[&invoice("receivedDebitNote")],
    );
    has(
        find(&xml, "Přijatý daňový doklad k platbě FV-47"),
        &["<int:intDoc ", "<typ:priceHigh>1000.00<"],
    );
    has(
        find(&xml, "Přijatý opravný doklad k platbě FV-48 k FV-47"),
        &["<int:intDoc ", "<typ:priceHigh>-100.00<"],
    );
    has(
        find(&xml, "Přijatý zjednodušený daňový doklad FV-49"),
        &[
            &invoice("receivedInvoice"),
            "<typ:priceNone>100.00</typ:priceNone>",
        ],
    );
    assert!(!xml.contains("accounting>"), "no codes set");
    assert_eq!(
        xml.matches("nonSubsume").count(),
        1,
        "only the non-deductible one"
    );
    has(
        find(&xml, "Zjednodušený daňový doklad ZJ-1"),
        &["<inv:roundingDocument>math2one</inv:roundingDocument>"],
    );

    // Direction filter.
    let issued = export(&app, &format!("{JANUARY}&direction=issued")).await;
    assert_eq!(items(&issued).len(), 8);
    assert!(!issued.contains("received"));
}

#[tokio::test]
async fn settings_codes_and_the_third_rate() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    import_all(&app, &fixture()).await;
    let (status, body) = call(
        &app,
        Method::PUT,
        "/api/settings/accounting",
        Some(json!({ "pohoda": { "ico": "12345679", "codes": [
            { "direction": "issued", "docType": "invoice", "accounting": "3Fv",
              "classificationVat": "UD", "numberSeries": "FV" },
            { "direction": "received", "docType": "advance_tax_doc", "accounting": "1Zal" },
            { "direction": "received", "docType": "invoice", "classificationVat": "PD",
              "classificationVatNonDeductible": "PN" },
        ]}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    add_rate_10(&app).await;

    let xml = export(&app, JANUARY).await;
    assert!(xml.contains(" ico=\"12345679\" "), "settings override");
    has(
        find(&xml, "Faktura FA-1"),
        &[
            "<inv:number><typ:ids>FV</typ:ids><typ:numberRequested>FA-1</typ:numberRequested></inv:number>",
            "<inv:accounting><typ:ids>3Fv</typ:ids></inv:accounting>",
            "<inv:classificationVAT><typ:ids>UD</typ:ids></inv:classificationVAT>",
            "<inv:originalDocument>FA-1</inv:originalDocument>",
        ],
    );
    let credit = find(&xml, "Dobropis DB-1 k FA-1");
    assert!(
        credit.contains("numberRequested") && !credit.contains("accounting>"),
        "{credit}"
    );
    has(
        find(&xml, "Přijatý daňový doklad k platbě FV-47"),
        &["<int:accounting><typ:ids>1Zal</typ:ids></int:accounting>"],
    );
    has(
        find(&xml, "Přijatá faktura FV-50"),
        &["<typ:price3>100.00</typ:price3><typ:price3VAT>10.00</typ:price3VAT>"],
    );
    has(
        find(&xml, "Přijatá faktura FV-42"),
        &["<inv:classificationVAT><typ:ids>PN</typ:ids>"],
    );
    has(
        find(&xml, "Přijatá faktura FV-44"),
        &["<inv:classificationVAT><typ:ids>PD</typ:ids>"],
    );
    assert_eq!(items(&xml).len(), 15, "16 imported − the proforma");
}

#[tokio::test]
async fn formats_and_an_empty_period() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let (status, body) = export_error(&app, &JANUARY.replace("pohoda", "abra")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["format"], "invalid");
    let (status, headers, _) =
        get_raw(&app, &JANUARY.replace("&format=pohoda", "&format=csv")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/csv; charset=utf-8");

    // No document: no file (an empty dataPack is not XSD-valid).
    let (status, body) = export_error(&app, JANUARY).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({ "code": "validation", "fields": { "from": "empty" } })
    );
}
