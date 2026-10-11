//! Round trip: documents → export → 2b import into a fresh schema →
//! export again reproduces every documented field (the received internal
//! number is allocated anew, so it is left out of the comparison).

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::csv_export::{Csv, export, export_bytes, fixture, import_all};
use common::documents::{create_bank, create_contact, create_issued, id, item, pay, set_company};
use common::received::{create_category, create_received};
use common::{TestDb, router};
use serde_json::json;

const ISSUED: &str = "/api/export/csv?direction=issued";
const RECEIVED: &str = "/api/export/csv?direction=received";

async fn company(app: &Router) {
    set_company(app, true).await;
    create_bank(app, "CZK").await;
}

/// The fixture plus native documents: a two-rate CZK invoice with rounding
/// paid in two payments, a partially paid invoice and a received invoice.
async fn source(app: &Router) {
    company(app).await;
    create_category(app, "Software", "expense").await;
    let customer = create_contact(
        app,
        json!({ "name": "Nativní Odběratel a.s.", "ico": "12345679", "dic": "CZ12345679",
                "street": "Dlouhá 5", "city": "Brno", "zip": "60200" }),
    )
    .await;
    let supplier = create_contact(
        app,
        json!({ "name": "Vzorový Dodavatel a.s.", "ico": "87654326" }),
    )
    .await;
    import_all(app, &fixture()).await;
    let lines = json!([item("1", "1000.20", "21"), item("1", "100", "12")]);
    let full = id(&create_issued(
        app,
        json!({ "contactId": customer, "issueDate": "2026-10-01", "lines": lines,
                "headerNote": "Nativní faktura", "roundingMode": "unit" }),
    )
    .await);
    for (date, amount) in [("2026-10-03", "1000"), ("2026-10-05", "322")] {
        let (status, p) = pay(app, &full, json!({ "date": date, "amount": amount })).await;
        assert_eq!(status, StatusCode::CREATED, "{p}");
    }
    let partial = id(&create_issued(
        app,
        json!({ "contactId": customer, "issueDate": "2026-10-02", "lines": [item("2", "50", "0")] }),
    )
    .await);
    let (status, p) = pay(app, &partial, json!({ "amount": "40" })).await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    create_received(
        app,
        &supplier,
        json!({ "receivedDate": "2026-10-02", "internalNote": "Interní poznámka",
                "variableSymbol": "777" }),
    )
    .await;
}

/// Rows with the received internal number blanked.
fn comparable(csv: &Csv) -> Vec<Vec<String>> {
    let direction = csv.header.iter().position(|h| h == "direction");
    let number = csv.header.iter().position(|h| h == "number");
    csv.rows
        .iter()
        .map(|r| {
            let mut r = r.clone();
            if let (Some(d), Some(n)) = (direction, number)
                && r[d] == "received"
            {
                r[n].clear();
            }
            r
        })
        .collect()
}

#[tokio::test]
async fn export_imports_back_unchanged() {
    let first = TestDb::new().await;
    let a = router(first.conn.clone());
    source(&a).await;
    let issued = export_bytes(&a, ISSUED).await;
    let received = export_bytes(&a, RECEIVED).await;
    let before = (export(&a, ISSUED).await, export(&a, RECEIVED).await);
    assert_eq!(before.0.rows.len(), 6);
    assert_eq!(before.1.rows.len(), 2);
    let native = before.0.find("note", "Nativní faktura");
    assert_eq!(before.0.cell(native, "rounding"), "-0,24");
    assert_eq!(before.0.cell(native, "total"), "1322,00");
    assert_eq!(before.0.cell(native, "paid_date"), "05.10.2026");
    assert_eq!(before.0.cell(native, "counterparty_street"), "Dlouhá 5");
    let partial = before.0.find("issue_date", "02.10.2026");
    assert_eq!(before.0.cell(partial, "paid_date"), "");

    let second = TestDb::new().await;
    let b = router(second.conn.clone());
    company(&b).await;
    import_all(&b, &issued).await;
    import_all(&b, &received).await;
    let after = (export(&b, ISSUED).await, export(&b, RECEIVED).await);

    for (x, y) in [(&before.0, &after.0), (&before.1, &after.1)] {
        assert_eq!(x.header, y.header);
        let (x, y) = (comparable(x), comparable(y));
        for (rx, ry) in x.iter().zip(&y) {
            assert_eq!(rx, ry);
        }
        assert_eq!(x.len(), y.len());
    }
    assert!(after.1.column("number").iter().all(|n| !n.is_empty()));
}
