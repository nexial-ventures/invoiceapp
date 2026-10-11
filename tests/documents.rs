//! Document drafts: create with defaults, validation, update, delete, compute.

mod common;

use axum::http::{Method, StatusCode};
use chrono::Days;
use common::documents::{create_bank, create_contact, create_doc, id, item, set_company};
use common::{TestDb, call, router};
use serde_json::{Value, json};

fn fields(body: &Value) -> &Value {
    &body["fields"]
}

#[tokio::test]
async fn create_applies_company_defaults() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let bank = create_bank(&app, "CZK").await;

    let doc = create_doc(&app, json!({ "lines": [item("2", "100", "21")] })).await;
    let today = invoice::time::today();
    assert_eq!(doc["status"], "draft");
    assert_eq!(doc["docType"], "invoice");
    assert_eq!(doc["direction"], "issued");
    assert_eq!(doc["number"], Value::Null);
    assert_eq!(doc["paymentState"], Value::Null);
    assert_eq!(doc["overdue"], false);
    assert_eq!(doc["issueDate"], today.to_string());
    assert_eq!(doc["taxPointDate"], today.to_string());
    let due = today.checked_add_days(Days::new(14)).expect("date");
    assert_eq!(doc["dueDate"], due.to_string());
    assert_eq!(doc["currency"], "CZK");
    assert_eq!(doc["locale"], "cs");
    assert_eq!(doc["vatMode"], "standard");
    assert_eq!(doc["paymentMethod"], "bank_transfer");
    assert_eq!(doc["bankAccountId"], bank);
    assert_eq!(doc["roundingMode"], "none");
    assert_eq!(doc["supplier"], Value::Null);
    assert_eq!(doc["customer"], Value::Null);
    assert_eq!(doc["paid"], "0.00");
    assert_eq!(
        doc["lines"],
        json!([{ "kind": "item", "position": 1, "description": "Práce", "quantity": "2",
                 "unit": "h", "unitPrice": "100", "discountPct": "0", "vatRate": "21",
                 "base": "200.00" }])
    );
    assert_eq!(
        doc["totals"],
        json!({ "recap": [{ "vatRate": "21", "base": "200.00", "vat": "42.00",
                            "baseCzk": null, "vatCzk": null }],
                "base": "200.00", "vat": "42.00", "total": "242.00", "rounding": "0.00",
                "payable": "242.00", "totalCzk": null })
    );

    let (status, got) = call(
        &app,
        Method::GET,
        &format!("/api/documents/{}", id(&doc)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, doc, "GET returns the stored values");
}

#[tokio::test]
async fn create_prefers_contact_defaults() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let contact = create_contact(
        &app,
        json!({ "defaultDueDays": 30, "defaultLocale": "en", "defaultCurrency": "EUR" }),
    )
    .await;
    let eur = create_bank(&app, "EUR").await;
    create_bank(&app, "CZK").await;

    let doc = create_doc(
        &app,
        json!({ "contactId": contact, "issueDate": "2026-10-01", "lines": [
            { "kind": "item", "description": "x", "quantity": "1", "unitPrice": "10" }
        ] }),
    )
    .await;
    assert_eq!(doc["dueDate"], "2026-10-31");
    assert_eq!(doc["taxPointDate"], "2026-10-01");
    assert_eq!(doc["locale"], "en");
    assert_eq!(doc["currency"], "EUR");
    assert_eq!(doc["bankAccountId"], eur);
    // The seeded company is not a VAT payer → non_payer and 0 % lines.
    assert_eq!(doc["vatMode"], "non_payer");
    assert_eq!(doc["lines"][0]["vatRate"], "0");
    assert_eq!(doc["totals"]["vat"], "0.00");
}

#[tokio::test]
async fn create_validates_with_line_indexes() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;

    let (status, body) = call(
        &app,
        Method::POST,
        "/api/documents",
        Some(json!({
            "docType": "credit_note",
            "contactId": "00000000-0000-0000-0000-000000000001",
            "currency": "EURO",
            "vatMode": "bogus",
            "variableSymbol": "12a",
            "constantSymbol": "12345",
            "lines": [
                item("1", "1", "21"),
                { "kind": "item", "description": "", "quantity": "0", "unitPrice": "1.23456" },
                { "kind": "text" }
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        fields(&body),
        &json!({
            "docType": "invalid", "contactId": "invalid", "currency": "invalid",
            "vatMode": "invalid", "variableSymbol": "invalid", "constantSymbol": "too_long",
            "lines.1.description": "required", "lines.1.quantity": "invalid",
            "lines.1.unitPrice": "invalid",
            "lines.2.description": "required"
        })
    );

    // Computation rules: subtotal refs, non-payer rates, an unknown rounding mode.
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/documents",
        Some(json!({
            "currency": "EUR",
            "roundingMode": "half",
            "lines": [
                item("1", "1", "21"), item("1", "1", "12"),
                { "kind": "subtotal", "description": "", "refs": [1, 2], "collapse": true }
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        fields(&body),
        &json!({ "lines.2.refs": "invalid" }),
        "rounding is allowed for a foreign currency"
    );
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/documents",
        Some(json!({ "roundingMode": "whole", "lines": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(fields(&body), &json!({ "roundingMode": "invalid" }));
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/documents",
        Some(
            json!({ "vatMode": "non_payer", "lines": [item("1", "1", "0"), item("1", "1", "21")] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(fields(&body), &json!({ "lines.1.vatRate": "invalid" }));

    let bank_eur = create_bank(&app, "EUR").await;
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/documents",
        Some(json!({ "bankAccountId": bank_eur, "lines": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(fields(&body), &json!({ "bankAccountId": "invalid" }));
}

#[tokio::test]
async fn update_replaces_a_draft() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let doc = create_doc(&app, json!({ "lines": [item("1", "100", "21")] })).await;
    let uri = format!("/api/documents/{}", id(&doc));

    let (status, body) = call(&app, Method::PUT, &uri, Some(json!({ "lines": [] }))).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "PUT applies no defaults"
    );
    assert_eq!(
        fields(&body),
        &json!({ "issueDate": "required", "dueDate": "required", "currency": "required",
                 "locale": "required", "vatMode": "required", "paymentMethod": "required" })
    );

    let (status, updated) = call(
        &app,
        Method::PUT,
        &uri,
        Some(json!({
            "issueDate": "2026-10-01", "dueDate": "2026-10-15", "currency": "czk", "locale": "en",
            "vatMode": "standard", "paymentMethod": "cash", "roundingMode": "unit",
            "internalNote": "  call first ",
            "lines": [
                item("1", "100.40", "21"),
                { "kind": "text", "description": "Poznámka" },
                { "kind": "item", "description": "Sleva", "quantity": "1", "unitPrice": "50",
                  "discountPct": "10", "vatRate": "12" },
                { "kind": "subtotal", "description": "Mezisoučet", "refs": [1], "collapse": false }
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["taxPointDate"], Value::Null);
    assert_eq!(updated["currency"], "CZK");
    assert_eq!(updated["paymentMethod"], "cash");
    assert_eq!(updated["internalNote"], "call first");
    assert_eq!(
        updated["lines"][1],
        json!({ "kind": "text", "position": 2, "description": "Poznámka" })
    );
    assert_eq!(updated["lines"][2]["base"], "45.00");
    assert_eq!(
        updated["lines"][3],
        json!({ "kind": "subtotal", "position": 4, "description": "Mezisoučet", "refs": [1],
                "collapse": false, "base": "100.40", "vatRate": "21" })
    );
    let t = &updated["totals"];
    // 100.40 + 21.08 + 45.00 + 5.40 = 171.88 → 172
    assert_eq!(
        (&t["total"], &t["payable"], &t["rounding"]),
        (&json!("171.88"), &json!("172.00"), &json!("0.12"))
    );
    assert_eq!(t["recap"].as_array().map(Vec::len), Some(2));
    assert_eq!(t["recap"][1]["vatRate"], "12");

    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/documents/00000000-0000-0000-0000-000000000001",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_draft_only() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let doc = create_doc(&app, json!({ "lines": [] })).await;
    let uri = format!("/api/documents/{}", id(&doc));
    let (status, _) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(&app, Method::GET, &uri, None).await;
    assert_eq!(
        (status, body),
        (StatusCode::NOT_FOUND, json!({ "code": "not_found" }))
    );
    let (status, _) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, Method::GET, "/api/documents/not-a-uuid", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn compute_endpoint_has_no_side_effects() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/documents/compute",
        Some(json!({
            "currency": "EUR", "exchangeRate": "25.125",
            "lines": [
                { "kind": "item", "description": "", "quantity": "1", "unitPrice": "100" },
                { "kind": "text" }
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["lines"][0]["vatRate"], "21",
        "default VAT rate fills the line"
    );
    assert_eq!(body["lines"][0]["base"], "100.00");
    assert_eq!(
        body["lines"][1],
        json!({ "kind": "text", "position": 2, "description": "" }),
        "descriptions are not validated"
    );
    assert_eq!(
        body["totals"]["recap"][0],
        json!({ "vatRate": "21", "base": "100.00", "vat": "21.00", "baseCzk": "2512.50", "vatCzk": "527.63" })
    );
    assert_eq!(body["totals"]["totalCzk"], "3040.13");

    let (status, body) = call(
        &app,
        Method::POST,
        "/api/documents/compute",
        Some(json!({ "vatMode": "non_payer", "lines": [item("1", "1", "21")] })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(fields(&body), &json!({ "lines.0.vatRate": "invalid" }));

    let (_, list) = call(&app, Method::GET, "/api/documents", None).await;
    assert_eq!(list["total"], 0);
}

#[tokio::test]
async fn malformed_bodies_are_bad_requests() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/documents",
        Some(json!({ "issueDate": "01.10.2026" })),
    )
    .await;
    assert_eq!(
        (status, body),
        (StatusCode::BAD_REQUEST, json!({ "code": "bad_request" }))
    );
}

#[tokio::test]
async fn overflowing_amounts_are_422_not_a_crash() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let huge = json!({ "currency": "EUR", "exchangeRate": "99999999999",
                       "lines": [item("1000000000", "1000000000", "21")] });
    for uri in ["/api/documents", "/api/documents/compute"] {
        let (status, body) = call(&app, Method::POST, uri, Some(huge.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}: {body}");
        assert_eq!(fields(&body), &json!({ "lines": "invalid" }), "{uri}");
    }
    let czk_only = json!({ "currency": "EUR", "exchangeRate": "99999999999",
                           "lines": [item("1000000", "1000000", "21")] });
    let (status, body) = call(&app, Method::POST, "/api/documents/compute", Some(czk_only)).await;
    assert_eq!(
        (status, fields(&body)),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "exchangeRate": "invalid" })
        )
    );
}
