//! Allocation takes the next free number after the counter
//! (`docs/api/numbering.md`): typos, imports and gaps.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{create_issued, issuable};
use common::{TestDb, call, router};
use serde_json::{Value, json};

fn with(body: &Value, patch: Value) -> Value {
    let mut b = body.clone();
    if let (Some(o), Some(p)) = (b.as_object_mut(), patch.as_object()) {
        o.extend(p.clone());
    }
    b
}

async fn set_counter(app: &axum::Router, last: i64) -> Value {
    let uri = "/api/settings/number-series/invoice/counters/2026";
    let (status, s) = call(app, Method::PUT, uri, Some(json!({ "lastNumber": last }))).await;
    assert_eq!(status, StatusCode::OK, "{s}");
    s
}

#[tokio::test]
async fn a_typo_manual_number_does_not_move_the_counter() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    assert_eq!(
        create_issued(&app, body.clone()).await["number"],
        "20260001"
    );
    let typo = create_issued(&app, with(&body, json!({ "number": "20269999" }))).await;
    assert_eq!(typo["number"], "20269999");
    assert_eq!(
        create_issued(&app, body.clone()).await["number"],
        "20260002"
    );
    assert_eq!(create_issued(&app, body).await["number"], "20260003");
}

#[tokio::test]
async fn an_imported_number_is_skipped() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let imported = with(&body, json!({ "imported": true, "number": "20260002" }));
    create_issued(&app, imported).await;
    assert_eq!(
        create_issued(&app, body.clone()).await["number"],
        "20260001"
    );
    assert_eq!(create_issued(&app, body).await["number"], "20260003");
}

#[tokio::test]
async fn a_lowered_counter_fills_gaps() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    for n in ["20260001", "20260002", "20260004"] {
        create_issued(&app, with(&body, json!({ "number": n }))).await;
    }
    let s = set_counter(&app, 0).await;
    // The settings preview (current year) shows the same next free number.
    if invoiceapp::time::current_year() == 2026 {
        assert_eq!(s["nextNumberPreview"], "20260003");
    }
    assert_eq!(
        create_issued(&app, body.clone()).await["number"],
        "20260003"
    );
    assert_eq!(create_issued(&app, body).await["number"], "20260005");
    let (_, all) = call(&app, Method::GET, "/api/settings/number-series", None).await;
    let invoice = all
        .as_array()
        .and_then(|a| a.iter().find(|s| s["docType"] == "invoice"))
        .cloned()
        .expect("invoice series");
    assert_eq!(
        invoice["counters"],
        json!([{ "year": 2026, "lastNumber": 5 }])
    );
}
