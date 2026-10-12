//! Settings → Accounting (`GET` / `PUT /api/settings/accounting`).

mod common;

use axum::http::{Method, StatusCode};
use common::{TestDb, call, get, router, send};
use serde_json::{Value, json};

const URI: &str = "/api/settings/accounting";

fn row(s: &Value, direction: &str, doc_type: &str) -> Value {
    section_row(s, "pohoda", direction, doc_type)
}

fn section_row(s: &Value, section: &str, direction: &str, doc_type: &str) -> Value {
    s[section]["codes"]
        .as_array()
        .expect("codes")
        .iter()
        .find(|r| r["direction"] == direction && r["docType"] == doc_type)
        .cloned()
        .unwrap_or_else(|| panic!("no row {direction} {doc_type} in {s}"))
}

#[tokio::test]
async fn responses_carry_the_code_limits() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let limits = json!({ "pohoda": { "code": 19, "numberSeries": 19 },
                         "money": { "code": 10, "numberSeries": 5 } });
    let (status, s) = call(&app, Method::GET, URI, None).await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["limits"], limits);
    let (status, saved) = call(&app, Method::PUT, URI, Some(json!({ "money": {} }))).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["limits"], limits);
}

#[tokio::test]
async fn get_put_and_validation() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());

    let resp = send(app.clone(), get(URI, None)).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let (status, s) = call(&app, Method::GET, URI, None).await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["pohoda"]["ico"], Value::Null);
    let codes = s["pohoda"]["codes"].as_array().expect("codes");
    assert_eq!(codes.len(), 12, "every direction × exported type");
    assert_eq!(
        codes[0],
        json!({ "direction": "issued", "docType": "invoice", "accounting": null,
                "classificationVat": null, "classificationVatNonDeductible": null, "numberSeries": null })
    );
    assert_eq!(s["money"]["ico"], Value::Null);
    assert_eq!(
        s["money"]["codes"].as_array().expect("money codes")[11],
        json!({ "direction": "received", "docType": "simplified", "accounting": null,
                "classificationVat": null, "classificationVatNonDeductible": null, "numberSeries": null })
    );

    let (status, saved) = call(
        &app,
        Method::PUT,
        URI,
        Some(json!({ "pohoda": { "ico": "87654326", "codes": [
            { "direction": "received", "docType": "simplified", "accounting": " 1Ppok ",
              "classificationVat": "", "classificationVatNonDeductible": "PN",
              "numberSeries": "PZ" },
        ]}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, s) = call(&app, Method::GET, URI, None).await;
    assert_eq!(s, saved);
    assert_eq!(s["pohoda"]["ico"], "87654326");
    assert_eq!(s["pohoda"]["codes"].as_array().expect("codes").len(), 12);
    assert_eq!(
        row(&s, "received", "simplified"),
        json!({ "direction": "received", "docType": "simplified", "accounting": "1Ppok",
                "classificationVat": null, "classificationVatNonDeductible": "PN", "numberSeries": "PZ" })
    );
    assert_eq!(row(&s, "issued", "simplified")["accounting"], Value::Null);

    let (status, err) = call(
        &app,
        Method::PUT,
        URI,
        Some(json!({ "pohoda": { "ico": "12345678", "codes": [
            { "direction": "sideways", "docType": "invoice" },
            { "direction": "issued", "docType": "proforma" },
            { "direction": "issued", "docType": "invoice", "numberSeries": "N".repeat(20) },
            { "direction": "issued", "docType": "invoice" },
            { "direction": "issued", "docType": "credit_note", "classificationVatNonDeductible": "PN" },
        ]}})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": {
            "pohoda.ico": "invalid_ico",
            "pohoda.codes.0.direction": "invalid",
            "pohoda.codes.1.docType": "invalid",
            "pohoda.codes.2.numberSeries": "too_long",
            "pohoda.codes.3.docType": "duplicate",
            "pohoda.codes.4.classificationVatNonDeductible": "invalid",
        }})
    );
    let (_, after) = call(&app, Method::GET, URI, None).await;
    assert_eq!(after, saved, "a rejected PUT changes nothing");

    // An empty object changes nothing (absent sections are kept); a
    // present empty section clears that one.
    let (status, same) = call(&app, Method::PUT, URI, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{same}");
    assert_eq!(same, saved);
    let (status, cleared) = call(&app, Method::PUT, URI, Some(json!({ "pohoda": {} }))).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["pohoda"]["ico"], Value::Null);
    assert_eq!(
        row(&cleared, "received", "simplified")["numberSeries"],
        Value::Null
    );
}

#[tokio::test]
async fn money_section() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let pohoda = json!({ "ico": "44444443", "codes": [
        { "direction": "issued", "docType": "invoice", "numberSeries": "FV" }] });
    let (status, saved) = call(
        &app,
        Method::PUT,
        URI,
        Some(
            json!({ "pohoda": pohoda, "money": { "ico": " 8765 4326 ", "codes": [
                { "direction": "received", "docType": "invoice", "accounting": "3Fp",
                  "classificationVat": "PD", "classificationVatNonDeductible": "PN",
                  "numberSeries": " FP " },
            ]}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["money"]["ico"], "87654326");
    assert_eq!(saved["money"]["codes"].as_array().expect("codes").len(), 12);
    assert_eq!(
        section_row(&saved, "money", "received", "invoice"),
        json!({ "direction": "received", "docType": "invoice", "accounting": "3Fp",
                "classificationVat": "PD", "classificationVatNonDeductible": "PN", "numberSeries": "FP" })
    );

    // Only Money sent: Pohoda kept; and the other way round.
    let (status, s) = call(
        &app,
        Method::PUT,
        URI,
        Some(json!({ "money": { "codes": [
            { "direction": "issued", "docType": "simplified", "numberSeries": "ZJ" }] }})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["pohoda"], saved["pohoda"], "absent section kept");
    assert_eq!(s["money"]["ico"], Value::Null, "present section replaced");
    assert_eq!(
        section_row(&s, "money", "received", "invoice")["accounting"],
        Value::Null
    );
    assert_eq!(
        section_row(&s, "money", "issued", "simplified")["numberSeries"],
        "ZJ"
    );
    let (status, s2) = call(&app, Method::PUT, URI, Some(json!({ "pohoda": {} }))).await;
    assert_eq!(status, StatusCode::OK, "{s2}");
    assert_eq!(s2["money"], s["money"], "Money kept");
    let (_, got) = call(&app, Method::GET, URI, None).await;
    assert_eq!(got, s2);

    let x = |n: usize| "x".repeat(n);
    let (status, err) = call(
        &app,
        Method::PUT,
        URI,
        Some(json!({ "money": { "ico": "12345678", "codes": [
            { "direction": "issued", "docType": "invoice", "accounting": x(11),
              "classificationVat": x(11), "numberSeries": x(6) },
            { "direction": "received", "docType": "invoice", "classificationVatNonDeductible": x(11) },
            { "direction": "issued", "docType": "debit_note", "classificationVatNonDeductible": "PN" },
            { "direction": "received", "docType": "invoice" },
            { "direction": "x", "docType": "proforma" },
        ]}, "pohoda": { "codes": [{ "direction": "issued", "docType": "invoice", "accounting": x(20) }] }})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": {
            "money.ico": "invalid_ico",
            "money.codes.0.accounting": "too_long",
            "money.codes.0.classificationVat": "too_long",
            "money.codes.0.numberSeries": "too_long",
            "money.codes.1.classificationVatNonDeductible": "too_long",
            "money.codes.2.classificationVatNonDeductible": "invalid",
            "money.codes.3.docType": "duplicate",
            "money.codes.4.direction": "invalid",
            "money.codes.4.docType": "invalid",
            "pohoda.codes.0.accounting": "too_long",
        }})
    );
    let (_, after) = call(&app, Method::GET, URI, None).await;
    assert_eq!(after, s2, "a rejected PUT changes nothing");
}
