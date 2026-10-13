//! Per-client number sequences: allocation, fallback, code changes, the
//! settings and contact routes, the read-only preview.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::create_contact;
use common::numbering::{
    CLIENT_PATTERN, client_setup, coded_contact, issue_for, set_client_pattern, set_code,
};
use common::{TestDb, call, router};
use serde_json::{Value, json};

async fn invoice_series(app: &axum::Router) -> Value {
    let (_, all) = call(app, Method::GET, "/api/settings/number-series", None).await;
    all.as_array()
        .and_then(|a| a.iter().find(|s| s["docType"] == "invoice"))
        .cloned()
        .expect("invoice series")
}

async fn counters(app: &axum::Router, contact: &str) -> Value {
    let uri = format!("/api/contacts/{contact}/number-counters");
    let (status, list) = call(app, Method::GET, &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list
}

async fn preview(app: &axum::Router, contact: Option<&str>) -> Value {
    let mut uri = "/api/documents/number-preview?docType=invoice&issueDate=2026-10-01".to_string();
    if let Some(c) = contact {
        uri.push_str(&format!("&contactId={c}"));
    }
    let (status, p) = call(app, Method::GET, &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    p
}

#[tokio::test]
async fn customers_with_a_code_get_their_own_sequence() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = client_setup(&app).await;
    let acme = coded_contact(&app, "Acme", Some("ACME")).await;

    assert_eq!(issue_for(&app, &body, &acme).await, "ACME-2026001");
    assert_eq!(issue_for(&app, &body, &acme).await, "ACME-2026002");
    // The global counter did not move.
    let series = invoice_series(&app).await;
    assert_eq!(series["clientPattern"], CLIENT_PATTERN);
    assert_eq!(series["counters"], json!([]));

    let list = counters(&app, &acme).await;
    assert_eq!(list.as_array().map(Vec::len), Some(1), "{list}");
    assert_eq!(list[0]["docType"], "invoice");
    assert_eq!(list[0]["clientPattern"], CLIENT_PATTERN);
    assert_eq!(
        list[0]["counters"],
        json!([{ "year": 2026, "lastNumber": 2 }])
    );
    let year = invoiceapp::time::current_year();
    let expected = if year == 2026 {
        "ACME-2026003".to_string()
    } else {
        format!("ACME-{year}001")
    };
    assert_eq!(list[0]["nextNumberPreview"], expected);
}

#[tokio::test]
async fn without_a_code_or_a_pattern_the_global_series_numbers() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = client_setup(&app).await;
    let plain = coded_contact(&app, "Plain", None).await;
    let acme = coded_contact(&app, "Acme", Some("ACME")).await;

    assert_eq!(issue_for(&app, &body, &plain).await, "20260001");
    // A customer's list has no preview without a code.
    assert_eq!(
        counters(&app, &plain).await[0]["nextNumberPreview"],
        Value::Null
    );
    // No client pattern: a code changes nothing.
    let (status, s) = set_client_pattern(&app, "invoice", "").await;
    assert_eq!(
        (status, &s["clientPattern"]),
        (StatusCode::OK, &Value::Null)
    );
    assert_eq!(issue_for(&app, &body, &acme).await, "20260002");
    assert_eq!(counters(&app, &acme).await, json!([]));
}

#[tokio::test]
async fn a_renamed_code_continues_its_counter() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = client_setup(&app).await;
    let acme = coded_contact(&app, "Acme", Some("ACME")).await;
    assert_eq!(issue_for(&app, &body, &acme).await, "ACME-2026001");

    let (status, c) = set_code(&app, &acme, "Acme", Some("ACX")).await;
    assert_eq!(status, StatusCode::OK, "{c}");
    assert_eq!(c["numberCode"], "ACX");
    assert_eq!(issue_for(&app, &body, &acme).await, "ACX-2026002");
}

#[tokio::test]
async fn a_reused_code_skips_the_numbers_issued_under_it() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = client_setup(&app).await;
    let old = coded_contact(&app, "Old", Some("ACME")).await;
    assert_eq!(issue_for(&app, &body, &old).await, "ACME-2026001");
    assert_eq!(issue_for(&app, &body, &old).await, "ACME-2026002");

    let (status, _) = set_code(&app, &old, "Old", None).await;
    assert_eq!(status, StatusCode::OK);
    // Stored as typed, matched case-insensitively.
    let new = coded_contact(&app, "New", Some("acme")).await;
    assert_eq!(issue_for(&app, &body, &new).await, "acme-2026003");
}

#[tokio::test]
async fn the_global_scope_ignores_client_numbers() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = client_setup(&app).await;
    // A client pattern that renders like the global one for code "2026".
    let (status, _) = set_client_pattern(&app, "invoice", "{C}{YYYY}{NNNN}").await;
    assert_eq!(status, StatusCode::OK);
    let acme = coded_contact(&app, "Acme", Some("ACME")).await;
    let plain = coded_contact(&app, "Plain", None).await;
    for n in 1..=3 {
        assert_eq!(
            issue_for(&app, &body, &acme).await,
            format!("ACME2026000{n}")
        );
    }
    assert_eq!(issue_for(&app, &body, &plain).await, "20260001");
}

#[tokio::test]
async fn a_lowered_client_counter_still_skips_issued_numbers() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = client_setup(&app).await;
    let acme = coded_contact(&app, "Acme", Some("ACME")).await;
    issue_for(&app, &body, &acme).await;
    issue_for(&app, &body, &acme).await;

    let uri = format!("/api/contacts/{acme}/number-counters/invoice/2026");
    let (status, item) = call(&app, Method::PUT, &uri, Some(json!({ "lastNumber": 0 }))).await;
    assert_eq!(status, StatusCode::OK, "{item}");
    assert_eq!(item["counters"], json!([{ "year": 2026, "lastNumber": 0 }]));
    assert_eq!(issue_for(&app, &body, &acme).await, "ACME-2026003");

    let (status, _) = call(&app, Method::PUT, &uri, Some(json!({ "lastNumber": 10 }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issue_for(&app, &body, &acme).await, "ACME-2026011");

    for (uri, body, field) in [
        (uri.clone(), json!({ "lastNumber": -1 }), "lastNumber"),
        (
            format!("/api/contacts/{acme}/number-counters/invoice/0"),
            json!({ "lastNumber": 1 }),
            "year",
        ),
        (
            format!("/api/contacts/{acme}/number-counters/received/2026"),
            json!({ "lastNumber": 1 }),
            "docType",
        ),
    ] {
        let (status, err) = call(&app, Method::PUT, &uri, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
        assert!(err["fields"].get(field).is_some(), "{err}");
    }
    let missing = format!("/api/contacts/{}/number-counters", uuid::Uuid::new_v4());
    assert_eq!(
        call(&app, Method::GET, &missing, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn client_pattern_validation() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    for (doc_type, pattern, reason) in [
        ("received", "{C}-P{YYYY}{NNN}", "not_issued"),
        ("invoice", "{YYYY}{NNN}", "invalid_pattern"),
        ("invoice", "{C}{C}{YYYY}{NNN}", "invalid_pattern"),
        ("invoice", "{C}-{NNN}", "invalid_pattern"),
    ] {
        let (status, err) = set_client_pattern(&app, doc_type, pattern).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{pattern}");
        assert_eq!(err["fields"]["clientPattern"], reason, "{pattern}");
    }
    // Another series' client pattern: both would issue the same numbers.
    let (status, s) = set_client_pattern(&app, "invoice", " {C}/{YY}/{NN} ").await;
    assert_eq!(
        (status, &s["clientPattern"]),
        (StatusCode::OK, &json!("{C}/{YY}/{NN}"))
    );
    let (status, err) = set_client_pattern(&app, "proforma", "{C}/{YY}/{NN}").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["fields"]["clientPattern"], "duplicate");
    // Re-saving its own is fine; clearing frees it.
    let (status, _) = set_client_pattern(&app, "invoice", "{C}/{YY}/{NN}").await;
    assert_eq!(status, StatusCode::OK);
    set_client_pattern(&app, "invoice", "").await;
    let (status, _) = set_client_pattern(&app, "proforma", "{C}/{YY}/{NN}").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn number_codes_are_validated_and_unique_case_insensitively() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    coded_contact(&app, "Zeta", Some("Acme")).await;
    let other = create_contact(&app, json!({ "name": "Other", "ico": null })).await;
    for (code, reason) in [
        ("acme", "duplicate"),
        ("AC ME", "invalid"),
        ("ABCDEFGHIJK", "invalid"),
    ] {
        let (status, err) = set_code(&app, &other, "Other", Some(code)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{code}");
        assert_eq!(err["fields"]["numberCode"], reason, "{code}");
    }
    let (status, c) = set_code(&app, &other, "Other", Some(" ")).await;
    assert_eq!((status, &c["numberCode"]), (StatusCode::OK, &Value::Null));
    // Searchable by code.
    let (_, list) = call(&app, Method::GET, "/api/contacts?q=acm", None).await;
    assert_eq!(list["total"], 1, "{list}");
}

#[tokio::test]
async fn preview_matches_the_issued_number_without_moving_a_counter() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = client_setup(&app).await;
    let acme = coded_contact(&app, "Acme", Some("ACME")).await;
    let plain = coded_contact(&app, "Plain", None).await;

    let expected = json!({ "number": "ACME-2026001", "source": "client" });
    assert_eq!(preview(&app, Some(&acme)).await, expected);
    assert_eq!(preview(&app, Some(&acme)).await, expected, "nothing moved");
    assert_eq!(counters(&app, &acme).await[0]["counters"], json!([]));
    assert_eq!(issue_for(&app, &body, &acme).await, "ACME-2026001");

    let global = json!({ "number": "20260001", "source": "global" });
    assert_eq!(preview(&app, Some(&plain)).await, global);
    assert_eq!(preview(&app, None).await, global);
    assert_eq!(issue_for(&app, &body, &plain).await, "20260001");

    let (status, err) = call(
        &app,
        Method::GET,
        "/api/documents/number-preview?docType=received",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["fields"]["docType"], "invalid");
    let uri = format!(
        "/api/documents/number-preview?contactId={}",
        uuid::Uuid::new_v4()
    );
    let (status, err) = call(&app, Method::GET, &uri, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["fields"]["contactId"], "invalid");
}
