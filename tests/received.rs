//! Received documents: lifecycle, numbering, ČNB rate, payments, list.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{
    create_contact, get_doc, hits, id, mock_cnb, pay, post_action, set_company,
};
use common::received::{create_received, received_body};
use common::{TestDb, call, router, router_with_cnb};
use serde_json::{Value, json};

async fn setup(app: &axum::Router) -> String {
    set_company(app, true).await;
    create_contact(
        app,
        json!({ "name": "Dodavatel Test s.r.o.", "ico": "27074358" }),
    )
    .await
}

#[tokio::test]
async fn create_put_delete() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let supplier = setup(&app).await;
    let doc = create_received(
        &app,
        &supplier,
        json!({ "vatRecap": [
            { "rate": "12", "base": "100", "vat": "12" },
            { "rate": "21", "base": "1000", "vat": "210" }
        ], "rounding": "-0.4", "payable": "1321.6", "variableSymbol": "2026777",
           "supplierAccount": "CZ65 0800 0000 1920 0014 5399", "internalNote": "pozn." }),
    )
    .await;
    assert_eq!(doc["direction"], "received");
    assert_eq!(doc["status"], "issued");
    assert_eq!(doc["number"], "P20260001");
    assert_eq!(doc["supplierNumber"], "FV-2026-777");
    assert_eq!(doc["receivedDate"], "2026-10-01");
    assert_eq!(doc["vatDeductible"], true);
    assert_eq!(doc["supplier"]["name"], "Dodavatel Test s.r.o.");
    assert_eq!(doc["customer"], Value::Null);
    assert_eq!(doc["lines"], json!([]));
    assert_eq!(
        doc["vatRecap"],
        json!([
            { "rate": "21", "base": "1000.00", "vat": "210.00" },
            { "rate": "12", "base": "100.00", "vat": "12.00" }
        ])
    );
    assert_eq!(
        (&doc["total"], &doc["rounding"], &doc["payable"]),
        (&json!("1321.60"), &json!("-0.40"), &json!("1321.60"))
    );
    assert_eq!(doc["totals"]["base"], "1100.00");
    // `totals` follows the issued convention; the top-level total adds rounding.
    assert_eq!(
        (
            &doc["totals"]["total"],
            &doc["totals"]["rounding"],
            &doc["totals"]["payable"]
        ),
        (&json!("1322.00"), &json!("-0.40"), &json!("1321.60"))
    );
    assert_eq!(doc["totals"]["vat"], "222.00");
    assert_eq!(doc["paymentState"], "unpaid");
    assert_eq!((&doc["sign"], &doc["original"]), (&json!(1), &Value::Null));
    assert_eq!(doc["customFields"], json!({}));

    // PUT any time: number fixed (also across years), snapshot refreshed.
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/contacts/{supplier}"),
        Some(json!({ "name": "Dodavatel Nový s.r.o.", "ico": "27074358" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = received_body(
        &supplier,
        json!({ "currency": "CZK", "vatMode": "standard", "receivedDate": "2027-01-05",
                "supplierNumber": "FV-2026-778", "vatDeductible": false }),
    );
    let (status, upd) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{}", id(&doc)),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{upd}");
    assert_eq!(upd["number"], "P20260001");
    assert_eq!(upd["supplierNumber"], "FV-2026-778");
    assert_eq!(upd["receivedDate"], "2027-01-05");
    assert_eq!(upd["vatDeductible"], false);
    assert_eq!(upd["supplier"]["name"], "Dodavatel Nový s.r.o.");
    assert_eq!(upd["payable"], "1210.00");

    // PUT cannot switch type or direction.
    let mut bad = received_body(
        &supplier,
        json!({ "currency": "CZK", "vatMode": "standard" }),
    );
    bad["docType"] = json!("proforma");
    let (status, e) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{}", id(&doc)),
        Some(bad.clone()),
    )
    .await;
    assert_eq!(
        (status, &e["fields"]["docType"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("invalid"))
    );
    bad["direction"] = json!("issued");
    let (status, e) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{}", id(&doc)),
        Some(bad),
    )
    .await;
    assert_eq!(
        (status, &e["fields"]["direction"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("invalid"))
    );

    for action in ["issue", "cancel", "mark-sent", "settle", "credit-note"] {
        let (status, e) = post_action(&app, &id(&doc), action).await;
        assert_eq!(
            (status, &e["code"]),
            (StatusCode::CONFLICT, &json!("invalid_state")),
            "{action}"
        );
    }

    let (status, p) = pay(&app, &id(&doc), json!({ "amount": "100" })).await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/documents/{}", id(&doc)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/documents/{}", id(&doc)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The number is not returned to the series.
    let next = create_received(&app, &supplier, json!({})).await;
    assert_eq!(next["number"], "P20260002");
}

#[tokio::test]
async fn numbers_per_type_and_received_year() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let s = setup(&app).await;
    let cn = create_received(&app, &s, json!({ "docType": "credit_note" })).await;
    assert_eq!(
        (&cn["number"], &cn["sign"]),
        (&json!("PD20260001"), &json!(-1))
    );
    let pf = create_received(
        &app,
        &s,
        json!({ "docType": "proforma", "taxPointDate": null, "currency": "EUR", "exchangeRate": "25" }),
    )
    .await;
    assert_eq!(pf["number"], "PZ20260001");
    assert_eq!(pf["receivedDate"], "2026-10-01");
    let ddpp = create_received(
        &app,
        &s,
        json!({ "docType": "advance_tax_doc", "dueDate": null, "relatedDocumentId": id(&pf) }),
    )
    .await;
    assert_eq!(ddpp["number"], "PDP20260001");
    assert_eq!(
        (&ddpp["dueDate"], &ddpp["overdue"]),
        (&Value::Null, &json!(false))
    );
    assert_eq!(ddpp["parent"]["id"], json!(id(&pf)));
    // Each link carries its own currency (CZK DDPP of a EUR proforma).
    assert_eq!(ddpp["parent"]["currency"], "EUR");
    let pf_now = common::documents::get_doc(&app, &id(&pf)).await;
    assert_eq!(pf_now["relatedDocuments"][0]["currency"], "CZK");
    let next_year = create_received(&app, &s, json!({ "receivedDate": "2027-01-03" })).await;
    assert_eq!(next_year["number"], "P20270001");
    // Native issued numbering is untouched.
    let (_, series) = call(&app, Method::GET, "/api/settings/number-series", None).await;
    let invoice = series
        .as_array()
        .expect("array")
        .iter()
        .find(|s| s["docType"] == "invoice")
        .cloned()
        .expect("invoice series");
    assert_eq!(invoice["counters"], json!([]));
    // Any counter value is accepted (allocation skips held numbers).
    let (status, e) = call(
        &app,
        Method::PUT,
        "/api/settings/number-series/received_proforma/counters/2026",
        Some(json!({ "lastNumber": 0 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{e}");
    // A received proforma cannot link anywhere; a credit note only to a received invoice.
    let body = received_body(
        &s,
        json!({ "docType": "credit_note", "relatedDocumentId": id(&pf) }),
    );
    let (status, e) = call(&app, Method::POST, "/api/documents", Some(body)).await;
    assert_eq!(
        (status, &e["fields"]["relatedDocumentId"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("invalid"))
    );
}

#[tokio::test]
async fn validation_errors() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let s = setup(&app).await;
    // The direction is trimmed before routing.
    let (status, d) = call(
        &app,
        Method::POST,
        "/api/documents",
        Some(received_body(&s, json!({ "direction": " received " }))),
    )
    .await;
    assert_eq!(
        (status, &d["direction"]),
        (StatusCode::CREATED, &json!("received"))
    );
    let body = received_body(
        &s,
        json!({ "supplierNumber": "", "vatMode": "exempt", "vatRecap": [
            { "rate": "21", "base": "100", "vat": "21" }, { "rate": "21", "base": "1", "vat": "0" }
        ], "payable": "-5", "rounding": "150" }),
    );
    let (status, e) = call(&app, Method::POST, "/api/documents", Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        e["fields"],
        json!({ "supplierNumber": "required", "vatRecap.0.vat": "invalid",
                "vatRecap.1.rate": "duplicate", "payable": "invalid", "rounding": "invalid" })
    );
    let body = received_body(&s, json!({ "vatRecap": [] , "contactId": null }));
    let (_, e) = call(&app, Method::POST, "/api/documents", Some(body)).await;
    assert_eq!(
        e["fields"],
        json!({ "vatRecap": "required", "contactId": "required" })
    );
}

#[tokio::test]
async fn cnb_rate_for_received_date() {
    let db = TestDb::new().await;
    let (url, counter) = mock_cnb().await;
    let app = router_with_cnb(db.conn.clone(), &url);
    let s = setup(&app).await;
    let doc = create_received(
        &app,
        &s,
        json!({ "currency": "EUR", "receivedDate": "2026-10-03",
                "vatRecap": [{ "rate": "21", "base": "100", "vat": "21" }], "payable": "121" }),
    )
    .await;
    assert_eq!(
        (
            &doc["exchangeRate"],
            &doc["exchangeRateSource"],
            &doc["exchangeRateDate"]
        ),
        (&json!("25.125"), &json!("cnb"), &json!("2026-10-03"))
    );
    assert_eq!(doc["totals"]["totalCzk"], "3040.13");
    assert_eq!(doc["totals"]["recap"][0]["baseCzk"], "2512.50");
    let calls = hits(&counter);
    let put = |extra: Value| {
        let mut b = received_body(
            &s,
            json!({ "currency": "EUR", "vatMode": "standard",
            "receivedDate": "2026-10-03", "vatRecap": [{ "rate": "21", "base": "100", "vat": "21" }],
            "payable": "121" }),
        );
        if let (Some(o), Some(e)) = (b.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        b
    };
    let uri = format!("/api/documents/{}", id(&doc));
    // Unchanged currency + date (rate echoed back): no new ČNB fetch, still "cnb".
    let (status, d) = call(
        &app,
        Method::PUT,
        &uri,
        Some(put(json!({ "exchangeRate": "25.125" }))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{d}");
    assert_eq!(
        (hits(&counter), &d["exchangeRateSource"]),
        (calls, &json!("cnb"))
    );
    // Manual override wins.
    let (_, d) = call(
        &app,
        Method::PUT,
        &uri,
        Some(put(json!({ "exchangeRate": "24" }))),
    )
    .await;
    assert_eq!(
        (&d["exchangeRate"], &d["exchangeRateSource"]),
        (&json!("24"), &json!("manual"))
    );
    // Date change → refetch for the new receivedDate.
    let (_, d) = call(
        &app,
        Method::PUT,
        &uri,
        Some(put(json!({ "receivedDate": "2026-10-04" }))),
    )
    .await;
    assert_eq!(
        (&d["exchangeRateSource"], &d["exchangeRateDate"]),
        (&json!("cnb"), &json!("2026-10-04"))
    );
    assert!(hits(&counter) > calls);
}

#[tokio::test]
async fn received_proforma_payments_never_issue_a_ddpp() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let s = setup(&app).await;
    let pf = create_received(
        &app,
        &s,
        json!({ "docType": "proforma", "taxPointDate": null }),
    )
    .await;
    let (status, p) = pay(&app, &id(&pf), json!({ "amount": "1210" })).await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    assert_eq!(p["advanceDocumentId"], Value::Null);
    let doc = get_doc(&app, &id(&pf)).await;
    assert_eq!(
        (&doc["paymentState"], &doc["relatedDocuments"]),
        (&json!("paid"), &json!([]))
    );
    let (_, list) = call(
        &app,
        Method::GET,
        "/api/documents?docType=advance_tax_doc",
        None,
    )
    .await;
    assert_eq!(list["total"], 0);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!(
            "/api/documents/{}/payments/{}",
            id(&pf),
            p["id"].as_str().expect("id")
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
