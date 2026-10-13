//! CSV list export (`GET /api/export/csv`): filters, exclusions, ordering,
//! column values, payments, rate columns, the empty file and the cap.

mod common;

use axum::http::StatusCode;
use common::csv_export::{export, export_bytes, export_error, fixture, get_raw, import_all};
use common::documents::{create_bank, create_contact, create_doc, create_issued, id, item, pay};
use common::documents::{post_action, set_company};
use common::received::create_category;
use common::{TestDb, router};
use sea_orm::ConnectionTrait;
use serde_json::json;

async fn setup(app: &axum::Router) -> String {
    set_company(app, true).await;
    create_bank(app, "CZK").await;
    create_category(app, "Software", "expense").await;
    create_contact(
        app,
        json!({ "name": "Nativní Odběratel", "ico": "12345679" }),
    )
    .await
}

/// A native issued CZK invoice of 1210 (1000 + 21 %), dated 2026-10-01.
async fn native(app: &axum::Router, contact: &str) -> String {
    id(&create_issued(
        app,
        json!({ "contactId": contact, "issueDate": "2026-10-01", "lines": [item("10", "100", "21")] }),
    )
    .await)
}

#[tokio::test]
async fn list_export_filters_and_exclusions() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let contact = setup(&app).await;
    import_all(&app, &fixture()).await;
    let body =
        json!({ "contactId": contact, "issueDate": "2026-10-01", "lines": [item("1", "5", "21")] });
    create_doc(&app, body.clone()).await;
    let cancelled = id(&create_issued(&app, body).await);
    let (status, c) = post_action(&app, &cancelled, "cancel").await;
    assert_eq!(status, StatusCode::OK, "{c}");
    native(&app, &contact).await;

    let (status, headers, _) = get_raw(&app, "/api/export/csv?direction=issued").await;
    assert_eq!(status, StatusCode::OK);
    let today = invoiceapp::time::today();
    assert_eq!(
        headers["content-disposition"],
        format!("attachment; filename=\"doklady-issued-{today}.csv\"")
    );
    let csv = export(&app, "/api/export/csv?direction=issued&limit=1&offset=3").await;
    let numbers = csv.column("number");
    assert_eq!(numbers.len(), 5, "no draft, no cancelled, no paging");
    assert_eq!(
        &numbers[..4],
        ["FA-3", "ZF-1", "FA-1", "DB-1"],
        "by tax date"
    );
    assert_eq!(csv.cell(4, "issue_date"), "01.10.2026");
    assert!(csv.column("direction").iter().all(|d| *d == "issued"));

    let fa1 = csv.find("number", "FA-1");
    assert_eq!(csv.cell(fa1, "tax_date"), "15.01.2026");
    assert_eq!(
        csv.cell(fa1, "counterparty_name"),
        "Fiktivní Odběratel s.r.o."
    );
    assert_eq!(csv.cell(fa1, "counterparty_country"), "CZ");
    assert_eq!(csv.cell(fa1, "paid_date"), "27.01.2026");
    assert_eq!(csv.cell(fa1, "category"), "Služby");
    assert_eq!(csv.cell(fa1, "note"), "Děkujeme");
    assert_eq!(csv.cell(fa1, "vat_deductible"), "");
    let zf = csv.find("number", "ZF-1");
    assert_eq!(
        (csv.cell(zf, "doc_type"), csv.cell(zf, "tax_date")),
        ("proforma", "")
    );

    // Foreign currency: CZK recap, document-currency total.
    let fa3 = csv.find("number", "FA-3");
    assert_eq!(
        (csv.cell(fa3, "currency"), csv.cell(fa3, "exchange_rate")),
        ("EUR", "25")
    );
    assert_eq!(
        (csv.cell(fa3, "base_21"), csv.cell(fa3, "vat_21")),
        ("2500,00", "525,00")
    );
    assert_eq!(
        (csv.cell(fa3, "total"), csv.cell(fa3, "total_czk")),
        ("121,00", "3025,00")
    );
    assert_eq!(csv.cell(fa1, "exchange_rate"), "");

    // Credit notes are negative and name their original.
    let db1 = export(&app, "/api/export/csv?direction=issued&docType=credit_note").await;
    assert_eq!(db1.column("number"), ["DB-1"]);
    assert_eq!(db1.cell(0, "related_number"), "FA-1");
    for (col, v) in [
        ("base_21", "-100,00"),
        ("vat_21", "-21,00"),
        ("total", "-121,00"),
    ] {
        assert_eq!(db1.cell(0, col), v, "{col}");
    }
    assert_eq!(
        (db1.cell(0, "total_czk"), db1.cell(0, "rounding")),
        ("-121,00", "0,00")
    );

    for (query, expected) in [
        ("paymentState=paid", vec!["FA-1"]),
        ("from=2026-01-10&to=2026-01-31", vec!["ZF-1", "FA-1"]),
        ("q=foreign", vec!["FA-3"]),
        ("status=draft", vec![]),
        ("status=cancelled", vec![]),
    ] {
        let csv = export(&app, &format!("/api/export/csv?direction=issued&{query}")).await;
        assert_eq!(csv.column("number"), expected, "{query}");
    }

    let received = export(&app, "/api/export/csv?direction=received").await;
    assert_eq!(received.column("supplier_number"), ["FV-42"]);
    assert!(
        !received.cell(0, "number").is_empty(),
        "the internal number"
    );
    assert_eq!(received.cell(0, "received_date"), "22.01.2026");
    assert_eq!(received.cell(0, "counterparty_dic"), "CZ87654326");
    assert_eq!(received.cell(0, "exchange_rate"), "24,335");
    assert_eq!(received.cell(0, "total_czk"), "2944,54");
    assert_eq!(received.cell(0, "vat_deductible"), "0");
    assert_eq!(received.cell(0, "category"), "Software");

    for (query, field, reason) in [
        ("", "direction", "required"),
        ("?direction=both", "direction", "invalid"),
    ] {
        let (status, body) = export_error(&app, &format!("/api/export/csv{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["fields"][field], reason, "{body}");
    }
}

#[tokio::test]
async fn paid_date_needs_full_payment() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let contact = setup(&app).await;
    let doc = native(&app, &contact).await;
    let paid = || async {
        export(&app, "/api/export/csv?direction=issued")
            .await
            .cell(0, "paid_date")
            .to_string()
    };

    let (status, p) = pay(
        &app,
        &doc,
        json!({ "date": "2026-10-03", "amount": "1000" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    assert_eq!(paid().await, "", "partial exports unpaid");
    let (status, p) = pay(&app, &doc, json!({ "date": "2026-10-07", "amount": "300" })).await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    assert_eq!(
        paid().await,
        "07.10.2026",
        "the payment completing it (overpaid)"
    );
}

#[tokio::test]
async fn rate_columns_are_settings_and_used_rates() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    setup(&app).await;
    let empty = export_bytes(&app, "/api/export/csv?direction=received").await;
    let text = String::from_utf8(empty).expect("utf-8");
    assert!(
        text.starts_with("\u{feff}direction;doc_type;number;"),
        "{text}"
    );
    assert_eq!(text.matches("\r\n").count(), 1, "header only");
    assert!(text.contains(";vat_mode;base_21;vat_21;base_12;vat_12;base_0;rounding;"));

    // A received invoice using a rate missing from Settings (10.5 %).
    let csv_text = "direction;doc_type;supplier_number;issue_date;counterparty_name;\
                    counterparty_ico;base_21;vat_21;base_10_5;vat_10_5;total\r\n\
                    received;invoice;FV-10;20.01.2026;Vzorový Dodavatel a.s.;87654326;\
                    100;21;100;10,50;231,50\r\n";
    import_all(&app, csv_text.as_bytes()).await;
    let csv = export(&app, "/api/export/csv?direction=received").await;
    assert_eq!(
        csv.rate_columns(),
        [
            "base_21",
            "vat_21",
            "base_12",
            "vat_12",
            "base_10_5",
            "vat_10_5",
            "base_0"
        ]
    );
    assert_eq!(
        (csv.cell(0, "base_10_5"), csv.cell(0, "vat_10_5")),
        ("100,00", "10,50")
    );
    assert_eq!(csv.cell(0, "base_12"), "");
}

#[tokio::test]
async fn more_than_ten_thousand_is_refused() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    setup(&app).await;
    import_all(&app, &fixture()).await;
    // 10 000 copies of FA-1 + 3 other issued documents = 10 004.
    db.conn
        .execute_unprepared(
            "INSERT INTO documents SELECT r.* FROM documents d, generate_series(1, 10000) g, \
             LATERAL jsonb_populate_record(NULL::documents, to_jsonb(d) || \
             jsonb_build_object('id', gen_random_uuid(), 'number', 'C-' || g)) r \
             WHERE d.number = 'FA-1'",
        )
        .await
        .expect("clone documents");
    for uri in [
        "/api/export/csv?direction=issued",
        "/api/export/accountant?from=2026-01-01&to=2026-12-31",
    ] {
        let (status, body) = export_error(&app, uri).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
        assert_eq!(body["fields"]["filter"], "too_many", "{uri}");
    }
    db.conn
        .execute_unprepared("DELETE FROM documents WHERE number IN ('C-1', 'C-2', 'C-3', 'C-4')")
        .await
        .expect("delete copies");
    // Exactly the cap streams in many chunks.
    let csv = export(&app, "/api/export/csv?direction=issued").await;
    assert_eq!(csv.rows.len(), 10_000);
    // The copies carry `paid` but no payment row: settled without a payment.
    let paid: Vec<&str> = csv
        .column("paid_date")
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    assert_eq!(paid, ["27.01.2026"]);
}
