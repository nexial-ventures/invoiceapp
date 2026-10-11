//! Phase 4e: the company's document defaults (GET / PUT, validation, partial
//! body) and the defaults a new document takes from them (REST and MCP).

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use common::documents::{create_contact, create_doc, id, item, set_company};
use common::mcp::tool;
use common::{TestDb, call, router};
use serde_json::{Value, json};

/// The profile `set_company` writes, plus `extra`.
fn company_body(extra: Value) -> Value {
    let mut b = json!({
        "name": "Dodavatel s.r.o.", "ico": "44444443", "vatPayer": true,
        "defaultDueDays": 14, "defaultLocale": "cs"
    });
    if let (Some(o), Some(e)) = (b.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    b
}

async fn put_company(app: &Router, extra: Value) -> (StatusCode, Value) {
    call(
        app,
        Method::PUT,
        "/api/settings/company",
        Some(company_body(extra)),
    )
    .await
}

/// Cash → half, other → unit, EUR, notes in both locales (footer cs only).
async fn custom_defaults(app: &Router) {
    let (status, body) = put_company(
        app,
        json!({
            "defaultPaymentMethod": "cash", "defaultRoundingCash": "half",
            "defaultRoundingOther": "unit", "defaultCurrency": "EUR",
            "defaultHeaderNote": { "cs": "Záhlaví", "en": "Header" },
            "defaultFooterNote": { "cs": "Zápatí", "en": null }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn company_defaults_are_seeded_and_partially_updated() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (status, c) = call(&app, Method::GET, "/api/settings/company", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(c["defaultRoundingCash"], "unit");
    assert_eq!(c["defaultRoundingOther"], "none");
    assert_eq!(c["defaultPaymentMethod"], "bank_transfer");
    assert_eq!(c["defaultCurrency"], "CZK");
    let empty = json!({ "cs": null, "en": null });
    assert_eq!(
        (&c["defaultHeaderNote"], &c["defaultFooterNote"]),
        (&empty, &empty)
    );

    custom_defaults(&app).await;
    // A body without the new fields (the 4a client) keeps them.
    set_company(&app, true).await;
    let (_, c) = call(&app, Method::GET, "/api/settings/company", None).await;
    assert_eq!(c["defaultPaymentMethod"], "cash");
    assert_eq!(c["defaultRoundingCash"], "half");
    assert_eq!(c["defaultRoundingOther"], "unit");
    assert_eq!(c["defaultCurrency"], "EUR");
    assert_eq!(
        c["defaultHeaderNote"],
        json!({ "cs": "Záhlaví", "en": "Header" })
    );
    assert_eq!(
        c["defaultFooterNote"],
        json!({ "cs": "Zápatí", "en": null })
    );

    // One field at a time; a note object replaces both locales.
    let (status, c) = put_company(
        &app,
        json!({ "defaultCurrency": "usd", "defaultHeaderNote": { "en": "  Thanks  " } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{c}");
    assert_eq!(c["defaultCurrency"], "USD");
    assert_eq!(
        c["defaultHeaderNote"],
        json!({ "cs": null, "en": "Thanks" })
    );
    assert_eq!(c["defaultPaymentMethod"], "cash");
}

#[tokio::test]
async fn company_defaults_are_validated() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let long = "x".repeat(2001);
    let (status, body) = put_company(
        &app,
        json!({
            "defaultRoundingCash": "whole", "defaultRoundingOther": "tenth",
            "defaultPaymentMethod": "cheque", "defaultCurrency": "EURO",
            "defaultHeaderNote": { "cs": long, "en": "ok" },
            "defaultFooterNote": { "cs": "ok", "en": long }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["fields"],
        json!({
            "defaultRoundingCash": "invalid", "defaultRoundingOther": "invalid",
            "defaultPaymentMethod": "invalid", "defaultCurrency": "invalid",
            "defaultHeaderNote.cs": "too_long", "defaultFooterNote.en": "too_long"
        })
    );
    // Nothing was stored.
    let (_, c) = call(&app, Method::GET, "/api/settings/company", None).await;
    assert_eq!(c["defaultRoundingCash"], "unit");
}

#[tokio::test]
async fn create_takes_the_company_defaults() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    // Seeded: bank transfer → none; cash → unit.
    let doc = create_doc(&app, json!({ "lines": [item("1", "100.40", "21")] })).await;
    assert_eq!(doc["paymentMethod"], "bank_transfer");
    assert_eq!(doc["roundingMode"], "none");
    assert_eq!(doc["totals"]["payable"], "121.48");
    let doc = create_doc(
        &app,
        json!({ "paymentMethod": "cash", "lines": [item("1", "100.40", "21")] }),
    )
    .await;
    assert_eq!(doc["roundingMode"], "unit");
    assert_eq!(
        (&doc["totals"]["payable"], &doc["totals"]["rounding"]),
        (&json!("121.00"), &json!("-0.48"))
    );

    custom_defaults(&app).await;
    let doc = create_doc(&app, json!({ "lines": [item("1", "100.40", "21")] })).await;
    assert_eq!(doc["paymentMethod"], "cash");
    assert_eq!(doc["roundingMode"], "half");
    assert_eq!(doc["currency"], "EUR");
    assert_eq!(doc["headerNote"], "Záhlaví");
    assert_eq!(doc["footerNote"], "Zápatí");
    assert_eq!(
        (&doc["totals"]["payable"], &doc["totals"]["rounding"]),
        (&json!("121.50"), &json!("0.02"))
    );
    let card = create_doc(&app, json!({ "paymentMethod": "card", "lines": [] })).await;
    assert_eq!(card["roundingMode"], "unit");

    // The contact's currency and locale win; the notes follow the locale.
    let contact = create_contact(
        &app,
        json!({ "defaultCurrency": "GBP", "defaultLocale": "en" }),
    )
    .await;
    let doc = create_doc(&app, json!({ "contactId": contact, "lines": [] })).await;
    assert_eq!(
        (&doc["currency"], &doc["locale"]),
        (&json!("GBP"), &json!("en"))
    );
    assert_eq!(doc["headerNote"], "Header");
    assert_eq!(doc["footerNote"], Value::Null);
    let plain = create_contact(&app, json!({ "name": "Bez výchozích s.r.o.", "ico": null })).await;
    let doc = create_doc(&app, json!({ "contactId": plain, "lines": [] })).await;
    assert_eq!(doc["currency"], "EUR");

    // Explicit values win, an explicit empty note stays empty.
    let doc = create_doc(
        &app,
        json!({ "roundingMode": "none", "headerNote": "", "footerNote": "Vlastní", "lines": [] }),
    )
    .await;
    assert_eq!(doc["roundingMode"], "none");
    assert_eq!(doc["headerNote"], Value::Null);
    assert_eq!(doc["footerNote"], "Vlastní");
}

#[tokio::test]
async fn put_applies_no_defaults() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    custom_defaults(&app).await;
    let doc = create_doc(&app, json!({ "lines": [] })).await;
    let (status, updated) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{}", id(&doc)),
        Some(json!({
            "issueDate": "2026-10-01", "dueDate": "2026-10-15", "currency": "CZK",
            "locale": "cs", "vatMode": "standard", "paymentMethod": "cash",
            "lines": [item("1", "100.40", "21")]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["roundingMode"], "none");
    assert_eq!(updated["headerNote"], Value::Null);
    assert_eq!(updated["footerNote"], Value::Null);
    assert_eq!(updated["totals"]["payable"], "121.48");
}

#[tokio::test]
async fn compute_takes_the_rounding_mode() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let compute = |mode: Value| {
        let app = app.clone();
        async move {
            call(
                &app,
                Method::POST,
                "/api/documents/compute",
                Some(json!({ "currency": "EUR", "roundingMode": mode,
                             "lines": [item("1", "100.20", "21")] })),
            )
            .await
        }
    };
    // 100.20 + 21.04 = 121.24
    let (status, body) = compute(json!("half")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totals"]["payable"], "121.00");
    let (_, body) = compute(Value::Null).await;
    assert_eq!(body["totals"]["payable"], "121.24", "default none");
    let (status, body) = compute(json!("ceil")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"], json!({ "roundingMode": "invalid" }));
}

#[tokio::test]
async fn mcp_draft_takes_defaults_and_the_mode() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    custom_defaults(&app).await;
    let draft = tool(
        &app,
        "create_draft",
        json!({ "lines": [item("1", "100.40", "21")] }),
    )
    .await;
    assert_eq!(draft["paymentMethod"], "cash");
    assert_eq!(draft["roundingMode"], "half");
    assert_eq!(draft["headerNote"], "Záhlaví");
    let draft = tool(
        &app,
        "create_draft",
        json!({ "currency": "CZK", "roundingMode": "unit", "lines": [item("1", "100.40", "21")] }),
    )
    .await;
    assert_eq!(draft["roundingMode"], "unit");
    assert_eq!(draft["totals"]["payable"], "121.00");
    let updated = tool(
        &app,
        "update_draft",
        json!({
            "id": draft["id"], "issueDate": "2026-10-01", "dueDate": "2026-10-15",
            "currency": "CZK", "locale": "cs", "vatMode": "standard",
            "paymentMethod": "bank_transfer", "roundingMode": "half",
            "lines": [item("1", "100.40", "21")]
        }),
    )
    .await;
    assert_eq!(updated["roundingMode"], "half");
    assert_eq!(updated["totals"]["payable"], "121.50");
}
