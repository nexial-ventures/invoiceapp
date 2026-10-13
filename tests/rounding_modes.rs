//! Phase 4e rounding modes end to end: issue / PDF / ISDOC with `half`, the
//! generators inheriting the mode, received documents and imports deriving
//! it from their amounts, and the `round_total` migration.

mod common;

use axum::http::{Method, StatusCode};
use common::csvio::{HEADER, confirm, doc_id, file, row};
use common::documents::{create_issued, get_doc, id, issuable, issue, item, pay, set_company};
use common::isdoc::unzip;
use common::mdcast::{PdfEnv, get_raw};
use common::received::create_received;
use common::{TestDb, call, router};
use invoiceapp::migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
use serde_json::json;

#[tokio::test]
async fn half_rounding_issues_prints_and_exports() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let mut body = issuable(&app).await;
    // 100.40 + 21.08 = 121.48 → 121.50
    body["lines"] = json!([item("1", "100.40", "21")]);
    body["roundingMode"] = json!("half");
    let doc = create_issued(&app, body).await;
    assert_eq!(doc["roundingMode"], "half");
    assert_eq!(
        (
            &doc["totals"]["total"],
            &doc["totals"]["rounding"],
            &doc["totals"]["payable"]
        ),
        (&json!("121.48"), &json!("0.02"), &json!("121.50"))
    );

    let totals = env.mock.last().data["totals"].clone();
    let values: Vec<_> = totals
        .as_array()
        .expect("totals")
        .iter()
        .map(|t| t["value"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(values.contains(&"0,02\u{a0}Kč".to_string()), "{values:?}");
    assert_eq!(values.last().map(String::as_str), Some("121,50\u{a0}Kč"));

    let number = doc["number"].as_str().expect("number");
    let (status, _, bytes) = get_raw(&app, &format!("/api/documents/{}/isdoc", id(&doc))).await;
    assert_eq!(status, StatusCode::OK);
    let files = unzip(&bytes);
    let (_, xml) = files
        .iter()
        .find(|(n, _)| *n == format!("{number}.isdoc"))
        .expect("isdoc entry");
    let xml = String::from_utf8_lossy(xml);
    assert!(
        xml.contains("<PayableRoundingAmount>0.02</PayableRoundingAmount>"),
        "{xml}"
    );
    assert!(
        xml.contains("<PayableAmount>121.50</PayableAmount>"),
        "{xml}"
    );
}

#[tokio::test]
async fn credit_note_and_ddpp_inherit_the_mode() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mut body = issuable(&app).await;
    body["lines"] = json!([item("1", "100.40", "21")]);
    body["roundingMode"] = json!("half");
    let invoice = create_issued(&app, body.clone()).await;
    let (status, cn) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{}/credit-note", id(&invoice)),
        Some(json!({ "correctionReason": "Vrácení" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{cn}");
    assert_eq!(cn["roundingMode"], "half");
    // Stored positive like every credit note (the sign is applied on output): 121.48 → 121.50.
    let (status, cn) = issue(&app, &id(&cn)).await;
    assert_eq!(status, StatusCode::OK, "{cn}");
    assert_eq!(
        (&cn["totals"]["rounding"], &cn["totals"]["payable"]),
        (&json!("0.02"), &json!("121.50"))
    );

    body["docType"] = json!("proforma");
    body["roundingMode"] = json!("unit");
    let proforma = create_issued(&app, body).await;
    assert_eq!(proforma["totals"]["payable"], "121.00");
    let (status, payment) = pay(&app, &id(&proforma), json!({ "amount": "121" })).await;
    assert_eq!(status, StatusCode::CREATED, "{payment}");
    let ddpp = get_doc(&app, payment["advanceDocumentId"].as_str().expect("DDPP")).await;
    assert_eq!(ddpp["roundingMode"], "none", "a DDPP is never rounded");
    assert_eq!(
        (&ddpp["totals"]["rounding"], &ddpp["totals"]["payable"]),
        (&json!("0.00"), &json!("121.00")),
        "the DDPP's amounts are the payment"
    );
    // Its correction inherits `none`: it nets against the unrounded DDPP.
    let (status, corr) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{}/credit-note", id(&ddpp)),
        Some(json!({ "correctionReason": "Vrácení zálohy" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{corr}");
    assert_eq!(corr["roundingMode"], "none");
}

#[tokio::test]
async fn received_documents_derive_the_mode() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    let supplier = common::documents::create_contact(&app, json!({})).await;
    let recap = |base: &str, vat: &str| json!([{ "rate": "21", "base": base, "vat": vat }]);
    let cases = [
        // (base, vat, rounding, mode)
        ("1000", "210", "0", "none"),
        ("100.40", "21.08", "-0.48", "unit"),
        ("100.40", "21.08", "0.02", "half"),
        ("100.40", "21.08", "0.01", "none"),
    ];
    for (i, (base, vat, rounding, mode)) in cases.into_iter().enumerate() {
        let doc = create_received(
            &app,
            &supplier,
            json!({ "supplierNumber": format!("FP-{i}"), "vatRecap": recap(base, vat),
                    "rounding": rounding, "payable": "1" }),
        )
        .await;
        assert_eq!(doc["roundingMode"], mode, "{base} {vat} {rounding}");
        assert_eq!(
            doc["rounding"],
            rounding_2dp(rounding),
            "the amount is kept"
        );
    }

    let rows = [
        row(&[
            ("direction", "received"),
            ("doc_type", "invoice"),
            ("supplier_number", "FV-HALF"),
            ("issue_date", "2026-01-20"),
            ("counterparty_name", "Vzorový Dodavatel a.s."),
            ("counterparty_ico", "87654326"),
            ("base_21", "100,40"),
            ("vat_21", "21,08"),
            ("rounding", "0,02"),
            ("total", "121,50"),
        ]),
        row(&[
            ("direction", "issued"),
            ("doc_type", "invoice"),
            ("number", "2026000101"),
            ("issue_date", "15.01.2026"),
            ("counterparty_name", "Fiktivní Odběratel s.r.o."),
            ("counterparty_ico", "12345679"),
            ("currency", "EUR"),
            ("exchange_rate", "25"),
            ("base_21", "2510,00"),
            ("vat_21", "527,00"),
            ("rounding", "-0,48"),
            ("total", "121,00"),
        ]),
    ];
    let bytes = file(HEADER, &[&rows[0], &rows[1]]);
    let results = confirm(&app, &bytes, &["row:2", "row:3"]).await;
    let received = get_doc(&app, &doc_id(&results, "row:2")).await;
    assert_eq!(received["roundingMode"], "half");
    let issued = get_doc(&app, &doc_id(&results, "row:3")).await;
    assert_eq!(issued["roundingMode"], "unit", "{issued}");
    assert_eq!(issued["totals"]["rounding"], "-0.48");
}

fn rounding_2dp(s: &str) -> String {
    format!("{:.2}", s.parse::<f64>().expect("number"))
}

#[tokio::test]
async fn migration_maps_round_total_to_the_mode() {
    let url = common::test_database_url();
    let schema = common::unique_schema();
    let admin = Database::connect(&url).await.expect("connect");
    admin
        .execute_unprepared(&format!("CREATE SCHEMA \"{schema}\""))
        .await
        .expect("schema");
    let mut opts = ConnectOptions::new(url.clone());
    opts.set_schema_search_path(schema.clone())
        .max_connections(2)
        .sqlx_logging(false);
    let conn = Database::connect(opts).await.expect("connect schema");
    // Everything before 4e (13 migrations), then rows as a 4d instance has them.
    Migrator::up(&conn, Some(13))
        .await
        .expect("pre-4e migrations");
    conn.execute_unprepared(
        "INSERT INTO spaces (id, slug, name) VALUES ('00000000-0000-0000-0000-00000000000a', 'm', 'M');
         INSERT INTO company (space_id) VALUES ('00000000-0000-0000-0000-00000000000a');
         INSERT INTO documents (id, space_id, direction, doc_type, imported, issue_date, currency,
                                locale, vat_mode, payment_method, round_total, total, rounding)
         VALUES
         ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-00000000000a',
          'issued', 'invoice', false, '2026-10-01', 'CZK', 'cs', 'standard', 'cash', true, 121.48, -0.48),
         ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-00000000000a',
          'issued', 'invoice', false, '2026-10-01', 'CZK', 'cs', 'standard', 'cash', false, 121.48, 0),
         ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-00000000000a',
          'received', 'invoice', false, '2026-10-01', 'CZK', 'cs', 'standard', 'bank_transfer', false, 121.48, 0.02),
         ('00000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-00000000000a',
          'received', 'invoice', false, '2026-10-01', 'EUR', 'cs', 'standard', 'bank_transfer', false, 121.48, -0.48),
         ('00000000-0000-0000-0000-000000000005', '00000000-0000-0000-0000-00000000000a',
          'received', 'invoice', false, '2026-10-01', 'CZK', 'cs', 'standard', 'bank_transfer', false, 121.48, 0.01),
         ('00000000-0000-0000-0000-000000000006', '00000000-0000-0000-0000-00000000000a',
          'issued', 'invoice', true, '2026-10-01', 'EUR', 'cs', 'standard', 'bank_transfer', false, 121.48, 0.02),
         ('00000000-0000-0000-0000-000000000007', '00000000-0000-0000-0000-00000000000a',
          'issued', 'invoice', true, '2026-10-01', 'CZK', 'cs', 'standard', 'cash', true, 121.00, 0),
         ('00000000-0000-0000-0000-000000000008', '00000000-0000-0000-0000-00000000000a',
          'issued', 'invoice', false, '2026-10-01', 'EUR', 'cs', 'standard', 'cash', false, 121.48, 0.02);
         UPDATE documents SET status = 'issued' WHERE id <> '00000000-0000-0000-0000-000000000007';",
    )
    .await
    .expect("4d rows");
    // Exactly 4e: later migrations would sit on top of it for the `down` below.
    Migrator::up(&conn, Some(1)).await.expect("4e migration");

    let rows = conn
        .query_all(Statement::from_string(
            conn.get_database_backend(),
            "SELECT rounding_mode FROM documents ORDER BY id",
        ))
        .await
        .expect("documents");
    let modes: Vec<String> = rows
        .iter()
        .map(|r| r.try_get("", "rounding_mode").expect("mode"))
        .collect();
    // 7: an imported draft's explicit flag wins over its (unrounded) amounts.
    assert_eq!(
        modes,
        [
            "unit", "none", "half", "unit", "none", "half", "unit", "none"
        ]
    );
    // 8: a native EUR document can now be `half` — it goes back as `false`.
    conn.execute_unprepared(
        "UPDATE documents SET rounding_mode = 'half' WHERE id = '00000000-0000-0000-0000-000000000008'",
    )
    .await
    .expect("half");
    let c = conn
        .query_one(Statement::from_string(
            conn.get_database_backend(),
            "SELECT default_rounding_cash, default_rounding_other, default_payment_method, \
             default_currency FROM company",
        ))
        .await
        .expect("company")
        .expect("row");
    let col = |n: &str| c.try_get::<String>("", n).expect(n);
    assert_eq!(
        [
            col("default_rounding_cash"),
            col("default_rounding_other"),
            col("default_payment_method"),
            col("default_currency")
        ],
        ["unit", "none", "bank_transfer", "CZK"]
    );
    // And back down.
    Migrator::down(&conn, Some(1)).await.expect("down");
    let rows = conn
        .query_all(Statement::from_string(
            conn.get_database_backend(),
            "SELECT round_total FROM documents ORDER BY id",
        ))
        .await
        .expect("documents");
    let flags: Vec<bool> = rows
        .iter()
        .map(|r| r.try_get("", "round_total").expect("flag"))
        .collect();
    // Only issued native CZK `unit` rows (1) keep the flag.
    assert_eq!(
        flags,
        [true, false, false, false, false, false, false, false]
    );
    conn.close().await.expect("close");
    admin
        .execute_unprepared(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .await
        .expect("drop schema");
}
