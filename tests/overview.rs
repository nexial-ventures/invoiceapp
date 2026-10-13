//! Overview (`GET /api/overview`): period figures by tax date (issued-status,
//! no proformas, CZK), VAT per rate, monthly series, categories, today's
//! receivables / payables, the non-payer basis and period validation.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{
    create_bank, create_contact, create_doc, create_issued, id, item, pay, set_company,
};
use common::received::{create_category, create_received};
use common::{TestApp, TestDb, call};
use rust_decimal::Decimal;
use serde_json::{Value, json};

const JANUARY: &str = "/api/overview?from=2026-01-01&to=2026-01-31";

fn dec(v: &Value) -> Decimal {
    v.as_str()
        .unwrap_or_else(|| panic!("decimal string: {v}"))
        .parse()
        .expect("decimal")
}

async fn overview(app: &axum::Router, uri: &str) -> Value {
    let (status, body) = call(app, Method::GET, uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn payer_figures_and_open_documents() {
    let db = TestDb::new().await;
    let app = TestApp::builder(&db.conn).build();
    set_company(&app, true).await;
    create_bank(&app, "CZK").await;
    let dev = create_category(&app, "Vývoj", "income").await;
    let customer = create_contact(&app, json!({ "name": "Odběratel a.s." })).await;
    let supplier = create_contact(&app, json!({ "name": "Dodavatel", "ico": "87654326" })).await;
    let issued = |date: &str, line: Value| json!({ "contactId": customer, "issueDate": date, "taxPointDate": date, "lines": [line] });

    // Unpaid and overdue (due 14 days after 2026-01-10).
    let open = create_issued(&app, issued("2026-01-10", item("10", "100", "21"))).await;
    let mut paid = issued("2026-01-20", item("1", "500", "12"));
    paid["categoryId"] = json!(dev);
    let paid = create_issued(&app, paid).await;
    let (status, p) = pay(&app, &id(&paid), json!({ "amount": "560" })).await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    // December: in the chart, not in the period; a draft and a proforma never.
    // Still unpaid: receivables are not bound to the period.
    let december = create_issued(&app, issued("2025-12-15", item("1", "300", "21"))).await;
    create_doc(&app, issued("2026-01-11", item("1", "7777", "21"))).await;
    let proforma = json!({ "docType": "proforma", "contactId": customer, "issueDate": "2026-01-12",
                           "lines": [item("1", "100", "21")] });
    create_issued(&app, proforma).await;

    create_received(
        &app,
        &supplier,
        json!({ "supplierNumber": "FV-1", "issueDate": "2026-01-05", "taxPointDate": "2026-01-05",
                "dueDate": "2026-01-20" }),
    )
    .await;
    create_received(
        &app,
        &supplier,
        json!({ "supplierNumber": "FV-2", "issueDate": "2026-01-06", "taxPointDate": "2026-01-06",
                "dueDate": "2026-01-21", "vatDeductible": false,
                "vatRecap": [{ "rate": "12", "base": "100", "vat": "12" }], "payable": "112" }),
    )
    .await;

    let o = overview(&app, JANUARY).await;
    assert_eq!(
        (o["from"].as_str(), o["to"].as_str()),
        (Some("2026-01-01"), Some("2026-01-31"))
    );
    assert_eq!(o["vatPayer"], true);
    assert_eq!(o["income"]["count"], 2);
    assert_eq!(dec(&o["income"]["amount"]), Decimal::from(1500));
    assert_eq!(dec(&o["income"]["vat"]), Decimal::from(270));
    assert_eq!(dec(&o["income"]["total"]), Decimal::from(1770));
    // Base 1000 + the non-deductible 112 in full.
    assert_eq!(dec(&o["expense"]["amount"]), Decimal::from(1112));
    assert_eq!(dec(&o["difference"]), Decimal::from(388));

    let vat = &o["vat"];
    let rows = |v: &Value| {
        v.as_array()
            .expect("rows")
            .iter()
            .map(|r| (dec(&r["rate"]), dec(&r["base"]), dec(&r["vat"])))
            .collect::<Vec<_>>()
    };
    let n = Decimal::from;
    assert_eq!(
        rows(&vat["output"]),
        [(n(21), n(1000), n(210)), (n(12), n(500), n(60))]
    );
    assert_eq!(rows(&vat["input"]), [(n(21), n(1000), n(210))]);
    assert_eq!(dec(&vat["result"]), n(60));
    assert_eq!(dec(&vat["nonDeductible"]["vat"]), n(12));
    assert_eq!(dec(&vat["reverseChargeIssued"]), n(0));

    let r = &o["receivables"];
    assert_eq!(
        (r["count"].as_u64(), r["overdueCount"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(dec(&r["amount"]), n(1210 + 363));
    assert_eq!(
        r["topOverdue"][0]["id"], december["id"],
        "longest overdue first"
    );
    assert_eq!(r["topOverdue"][1]["id"], open["id"]);
    assert_eq!(r["topOverdue"][1]["counterparty"], "Odběratel a.s.");
    assert_eq!(r["topOverdue"][1]["dueDate"], "2026-01-24");
    assert_eq!(
        (
            r["advances"]["count"].as_u64(),
            dec(&r["advances"]["amount"])
        ),
        (Some(1), n(121))
    );
    let p = &o["payables"];
    assert_eq!(
        (p["count"].as_u64(), p["overdueCount"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(dec(&p["amount"]), n(1322));
    assert_eq!(
        p["topOverdue"][0]["number"], "FV-1",
        "the supplier's number"
    );

    let months = o["months"].as_array().expect("months");
    assert_eq!(months.len(), 12);
    assert_eq!(
        (
            months[11]["month"].as_str(),
            months[11]["inPeriod"].as_bool()
        ),
        (Some("2026-01"), Some(true))
    );
    assert_eq!(dec(&months[11]["income"]), n(1500));
    assert_eq!(dec(&months[11]["expense"]), n(1112));
    assert_eq!(
        (months[10]["month"].as_str(), dec(&months[10]["income"])),
        (Some("2025-12"), n(300))
    );
    assert_eq!(months[10]["inPeriod"], false);

    let income = o["categories"]["income"].as_array().expect("income");
    let got: Vec<_> = income
        .iter()
        .map(|c| (c["name"].as_str(), dec(&c["amount"])))
        .collect();
    assert_eq!(got, [(None, n(1000)), (Some("Vývoj"), n(500))]);

    // A quarter shows its own months; the December document is outside it.
    let q = overview(&app, "/api/overview?from=2026-01-01&to=2026-03-31").await;
    let keys: Vec<_> = q["months"]
        .as_array()
        .expect("months")
        .iter()
        .map(|m| m["month"].clone())
        .collect();
    assert_eq!(keys, [json!("2026-01"), json!("2026-02"), json!("2026-03")]);
    assert_eq!(q["income"]["count"], 2);
}

#[tokio::test]
async fn non_payer_totals_and_foreign_amounts() {
    let db = TestDb::new().await;
    let app = TestApp::builder(&db.conn).build();
    set_company(&app, false).await;
    create_bank(&app, "CZK").await;
    let customer = create_contact(&app, json!({})).await;
    let supplier =
        create_contact(&app, json!({ "name": "Supplier GmbH", "ico": "87654326" })).await;
    create_issued(
        &app,
        json!({ "contactId": customer, "issueDate": "2026-02-02",
                "lines": [item("2", "400", "0")] }),
    )
    .await;
    create_received(
        &app,
        &supplier,
        json!({ "supplierNumber": "EU-1", "issueDate": "2026-02-03", "taxPointDate": "2026-02-03",
                "currency": "EUR", "exchangeRate": "25", "dueDate": "2099-01-01",
                "vatRecap": [{ "rate": "21", "base": "100", "vat": "21" }], "payable": "121" }),
    )
    .await;
    let o = overview(&app, "/api/overview?from=2026-02-01&to=2026-02-28").await;
    assert_eq!(o["vatPayer"], false);
    assert_eq!(dec(&o["income"]["amount"]), Decimal::from(800));
    // Non-payer: the expense is the CZK total incl. VAT.
    assert_eq!(dec(&o["expense"]["amount"]), Decimal::from(3025));
    assert_eq!(dec(&o["expense"]["base"]), Decimal::from(2500));
    let p = &o["payables"];
    assert_eq!(
        (dec(&p["amount"]), p["overdueCount"].as_u64()),
        (Decimal::from(3025), Some(0))
    );
}

#[tokio::test]
async fn invalid_period() {
    let db = TestDb::new().await;
    let app = TestApp::builder(&db.conn).build();
    for (query, fields) in [
        ("", &["from", "to"][..]),
        ("from=2026-01-01", &["to"]),
        ("from=01.01.2026&to=2026-01-31", &["from"]),
        ("from=2026-02-01&to=2026-01-31", &["from", "to"]),
        ("from=2026-01-01&to=2027-01-03", &["from", "to"]),
    ] {
        let (status, body) = call(&app, Method::GET, &format!("/api/overview?{query}"), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
        assert_eq!(body["code"], "validation");
        let got = body["fields"].as_object().expect("fields");
        assert_eq!(got.len(), fields.len(), "{query}: {body}");
        for f in fields {
            assert_eq!(got[*f], "invalid", "{query}: {body}");
        }
    }
    // An empty space: zeros, one empty bar per month of the period.
    let o = overview(&app, "/api/overview?from=2027-03-01&to=2028-03-01").await;
    assert_eq!(o["months"].as_array().map(Vec::len), Some(13));
    assert_eq!(o["income"]["count"], 0);
}
