//! Number series settings and `allocate_number`.

mod common;

use axum::http::{Method, StatusCode};
use common::{TestDb, call, router};
use invoice::document::doc_type::DocType;
use invoice::settings::repo::number_series::allocate_number;
use sea_orm::TransactionTrait;
use serde_json::{Value, json};

fn find<'a>(list: &'a Value, doc_type: &str) -> &'a Value {
    list.as_array()
        .and_then(|l| l.iter().find(|s| s["docType"] == doc_type))
        .unwrap_or(&Value::Null)
}

#[tokio::test]
async fn series_are_seeded_with_preview() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let year = invoice::time::current_year();

    let (status, list) = call(&app, Method::GET, "/api/settings/number-series", None).await;
    assert_eq!(status, StatusCode::OK);
    let summary: Vec<_> = list
        .as_array()
        .expect("array")
        .iter()
        .map(|s| {
            (
                s["docType"].clone(),
                s["pattern"].clone(),
                s["counters"].clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (json!("invoice"), json!("{YYYY}{NNNN}"), json!([])),
            (json!("credit_note"), json!("D{YYYY}{NNNN}"), json!([])),
            (json!("debit_note"), json!("V{YYYY}{NNNN}"), json!([])),
            (json!("proforma"), json!("Z{YYYY}{NNNN}"), json!([])),
            (json!("advance_tax_doc"), json!("DP{YYYY}{NNNN}"), json!([])),
            (
                json!("advance_credit_note"),
                json!("OP{YYYY}{NNNN}"),
                json!([])
            ),
            (json!("simplified"), json!("ZD{YYYY}{NNNN}"), json!([])),
            (json!("received"), json!("P{YYYY}{NNNN}"), json!([])),
            (
                json!("received_credit_note"),
                json!("PD{YYYY}{NNNN}"),
                json!([])
            ),
            (
                json!("received_debit_note"),
                json!("PV{YYYY}{NNNN}"),
                json!([])
            ),
            (
                json!("received_proforma"),
                json!("PZ{YYYY}{NNNN}"),
                json!([])
            ),
            (
                json!("received_advance_tax_doc"),
                json!("PDP{YYYY}{NNNN}"),
                json!([])
            ),
            (
                json!("received_advance_credit_note"),
                json!("POP{YYYY}{NNNN}"),
                json!([])
            ),
            (
                json!("received_simplified"),
                json!("PZD{YYYY}{NNNN}"),
                json!([])
            ),
        ]
    );
    assert_eq!(
        find(&list, "credit_note")["nextNumberPreview"],
        format!("D{year}0001")
    );
}

#[tokio::test]
async fn pattern_update_and_validation() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let yy = invoice::time::current_year() % 100;

    let (status, s) = call(
        &app,
        Method::PUT,
        "/api/settings/number-series/proforma",
        Some(json!({ "pattern": " ZF-{YY}/{NNN} " })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["docType"], "proforma");
    assert_eq!(s["pattern"], "ZF-{YY}/{NNN}");
    assert_eq!(s["nextNumberPreview"], format!("ZF-{yy:02}/001"));

    for bad in ["{YYYY}", "A B{YY}{NNN}", "{YY}{N}{N}", "{NNN}", ""] {
        let (status, err) = call(
            &app,
            Method::PUT,
            "/api/settings/number-series/proforma",
            Some(json!({ "pattern": bad })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(
            err,
            json!({ "code": "validation", "fields": { "pattern": "invalid_pattern" } })
        );
    }

    for taken in ["{YYYY}{NNNN}", "D{YYYY}{NNNN}"] {
        let (status, err) = call(
            &app,
            Method::PUT,
            "/api/settings/number-series/proforma",
            Some(json!({ "pattern": taken })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{taken}");
        assert_eq!(
            err,
            json!({ "code": "validation", "fields": { "pattern": "duplicate" } })
        );
    }
    // Re-saving a series' own pattern is not a duplicate.
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/number-series/invoice",
        Some(json!({ "pattern": "{YYYY}{NNNN}" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/number-series/order",
        Some(json!({ "pattern": "{YY}{NNN}" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn counter_upsert() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let year = invoice::time::current_year();

    let uri = format!("/api/settings/number-series/invoice/counters/{year}");
    let (status, s) = call(&app, Method::PUT, &uri, Some(json!({ "lastNumber": 41 }))).await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["counters"], json!([{ "year": year, "lastNumber": 41 }]));
    assert_eq!(s["nextNumberPreview"], format!("{year}0042"));

    let (_, s) = call(&app, Method::PUT, &uri, Some(json!({ "lastNumber": 7 }))).await;
    assert_eq!(s["counters"], json!([{ "year": year, "lastNumber": 7 }]));

    let old = format!("/api/settings/number-series/invoice/counters/{}", year - 1);
    let (_, s) = call(&app, Method::PUT, &old, Some(json!({ "lastNumber": 0 }))).await;
    assert_eq!(
        s["counters"],
        json!([{ "year": year, "lastNumber": 7 }, { "year": year - 1, "lastNumber": 0 }])
    );

    let (status, err) = call(&app, Method::PUT, &uri, Some(json!({ "lastNumber": -1 }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["fields"], json!({ "lastNumber": "invalid" }));
    let (status, err) = call(
        &app,
        Method::PUT,
        "/api/settings/number-series/invoice/counters/0",
        Some(json!({ "lastNumber": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["fields"], json!({ "year": "invalid" }));
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/number-series/invoice/counters/abc",
        Some(json!({ "lastNumber": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn allocate_continues_from_counter_and_rolls_back() {
    let db = TestDb::new().await;
    let space = db.space;
    let app = router(db.conn.clone());
    call(
        &app,
        Method::PUT,
        "/api/settings/number-series/credit_note/counters/2025",
        Some(json!({ "lastNumber": 9 })),
    )
    .await;

    let txn = db.conn.begin().await.expect("begin");
    let n = allocate_number(&txn, space, DocType::CreditNote, 2025)
        .await
        .expect("allocate");
    assert_eq!(n, ("D20250010".to_string(), 10));
    txn.rollback().await.expect("rollback");

    let txn = db.conn.begin().await.expect("begin");
    assert_eq!(
        allocate_number(&txn, space, DocType::CreditNote, 2025)
            .await
            .expect("allocate")
            .0,
        "D20250010",
        "a rolled-back allocation returns its number"
    );
    assert_eq!(
        allocate_number(&txn, space, DocType::CreditNote, 2026)
            .await
            .expect("allocate"),
        ("D20260001".to_string(), 1),
        "a new year starts at 1"
    );
    txn.commit().await.expect("commit");
}

#[tokio::test]
async fn concurrent_allocations_are_distinct_and_consecutive() {
    let db = TestDb::new().await;
    let space = db.space;
    const N: usize = 20;

    let tasks: Vec<_> = (0..N)
        .map(|_| {
            let conn = db.conn.clone();
            tokio::spawn(async move {
                let txn = conn.begin().await.expect("begin");
                let n = allocate_number(&txn, space, DocType::Invoice, 2026)
                    .await
                    .expect("allocate")
                    .0;
                // Hold the row lock briefly so allocations really contend.
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                txn.commit().await.expect("commit");
                n
            })
        })
        .collect();
    let mut numbers = Vec::new();
    for t in tasks {
        numbers.push(t.await.expect("join"));
    }
    numbers.sort();
    let expected: Vec<String> = (1..=N).map(|i| format!("2026{i:04}")).collect();
    assert_eq!(numbers, expected);

    let (_, list) = call(
        &router(db.conn.clone()),
        Method::GET,
        "/api/settings/number-series",
        None,
    )
    .await;
    assert_eq!(
        find(&list, "invoice")["counters"],
        json!([{ "year": 2026, "lastNumber": N }])
    );
}

#[tokio::test]
async fn counter_guard_waits_for_an_issue_in_flight() {
    use sea_orm::ConnectionTrait;
    let db = TestDb::new().await;
    let space = db.space;
    let app = router(db.conn.clone());

    // An issue in flight: number allocated and document written, not committed.
    let txn = db.conn.begin().await.expect("begin");
    let (number, seq) = allocate_number(&txn, space, DocType::Invoice, 2026)
        .await
        .expect("allocate");
    txn.execute_unprepared(&format!(
        "INSERT INTO documents (id, space_id, direction, doc_type, status, number, number_year, \
         number_seq, issue_date, due_date, currency, locale, vat_mode, payment_method) VALUES \
         (gen_random_uuid(), '{space}', 'issued', 'invoice', 'issued', '{number}', 2026, {seq}, \
         '2026-10-01', '2026-10-15', 'CZK', 'cs', 'standard', 'bank_transfer')"
    ))
    .await
    .expect("insert document");

    let put = tokio::spawn({
        let app = app.clone();
        async move {
            call(
                &app,
                Method::PUT,
                "/api/settings/number-series/invoice/counters/2026",
                Some(json!({ "lastNumber": 0 })),
            )
            .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !put.is_finished(),
        "the guard must wait for the counter row lock"
    );
    txn.commit().await.expect("commit");

    let (status, body) = put.await.expect("join");
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["fields"], json!({ "lastNumber": "below_issued" }));
}
