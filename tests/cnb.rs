//! ČNB exchange rates (route, cache) and foreign-currency issue — against a
//! local mock; the real ČNB is never called.

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use chrono::Days;
use common::documents::{
    create_bank, create_contact, create_doc, dead_url, hits, id, issue, item, mock_cnb,
    mock_cnb_published, set_company,
};
use common::{TestDb, call, router_with_cnb};
use invoice::time::today;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};

async fn rate(app: &Router, uri: &str) -> (StatusCode, Value) {
    call(app, Method::GET, uri, None).await
}

#[tokio::test]
async fn rate_route_uses_the_cache() {
    let db = TestDb::new().await;
    let (url, counter) = mock_cnb().await;
    let app = router_with_cnb(db.conn.clone(), &url);

    let (status, body) = rate(&app, "/api/exchange-rates/eur?date=2026-10-01").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "currency": "EUR", "date": "2026-10-01", "rate": "25.125" })
    );
    let (_, jpy) = rate(&app, "/api/exchange-rates/JPY?date=2026-10-01").await;
    assert_eq!(jpy["rate"], "0.15512", "rate per 1 unit (množství 100)");
    assert_eq!(hits(&counter), 1, "the whole list is cached for the date");

    for _ in 0..2 {
        let (status, body) = rate(&app, "/api/exchange-rates/XYZ?date=2026-10-01").await;
        assert_eq!(
            (status, body),
            (StatusCode::NOT_FOUND, json!({ "code": "not_found" }))
        );
    }
    assert_eq!(
        hits(&counter),
        2,
        "a currency missing from a final list is cached as absent"
    );
    let (status, _) = rate(&app, "/api/exchange-rates/EURO").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = rate(&app, "/api/exchange-rates/CZK?date=2026-10-01").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["rate"], "1");
    let (status, _) = rate(&app, "/api/exchange-rates/EUR?date=1.10.2026").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unreachable_cnb_is_502() {
    let db = TestDb::new().await;
    let app = router_with_cnb(db.conn.clone(), &dead_url());
    let (status, body) = rate(&app, "/api/exchange-rates/EUR?date=2026-10-01").await;
    assert_eq!(
        (status, body),
        (
            StatusCode::BAD_GATEWAY,
            json!({ "code": "cnb_unavailable" })
        )
    );
}

#[tokio::test]
async fn a_list_that_may_still_change_is_not_cached() {
    let db = TestDb::new().await;
    let (url, counter) = mock_cnb_published(Some("01.10.2026")).await;
    let app = router_with_cnb(db.conn.clone(), &url);
    // Today or later, answered with an older list: today's may not be out yet.
    let today = today();
    for _ in 0..2 {
        let (status, body) = rate(&app, &format!("/api/exchange-rates/EUR?date={today}")).await;
        assert_eq!(
            (status, &body["date"]),
            (StatusCode::OK, &json!("2026-10-01"))
        );
    }
    assert_eq!(
        hits(&counter),
        1,
        "reused from memory for a few minutes, never stored"
    );
    let stored = db
        .conn
        .query_one(Statement::from_string(
            db.conn.get_database_backend(),
            format!("SELECT count(*) AS n FROM exchange_rates WHERE requested_date = '{today}'"),
        ))
        .await
        .expect("count")
        .and_then(|r| r.try_get::<i64>("", "n").ok());
    assert_eq!(stored, Some(0));
    // A past date (e.g. a weekend) answered with an older list is final.
    let past = today.checked_sub_days(Days::new(400)).expect("date");
    for _ in 0..2 {
        rate(&app, &format!("/api/exchange-rates/EUR?date={past}")).await;
    }
    assert_eq!(hits(&counter), 2);
}

async fn eur_draft(app: &Router, extra: Value) -> String {
    set_company(app, true).await;
    let contact = create_contact(app, json!({})).await;
    create_bank(app, "EUR").await;
    let mut body = json!({
        "contactId": contact, "currency": "EUR", "issueDate": "2026-10-01",
        "taxPointDate": "2026-09-30", "lines": [item("1", "100", "21")]
    });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    id(&create_doc(app, body).await)
}

#[tokio::test]
async fn foreign_issue_fetches_cnb_for_the_tax_point_date() {
    let db = TestDb::new().await;
    let (url, counter) = mock_cnb().await;
    let app = router_with_cnb(db.conn.clone(), &url);
    let first = eur_draft(&app, json!({})).await;
    let draft = call(&app, Method::GET, &format!("/api/documents/{first}"), None)
        .await
        .1;
    assert_eq!(
        draft["totals"]["totalCzk"],
        Value::Null,
        "no rate known yet"
    );

    let (status, doc) = issue(&app, &first).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["exchangeRate"], "25.125");
    assert_eq!(doc["exchangeRateDate"], "2026-09-30");
    assert_eq!(doc["exchangeRateSource"], "cnb");
    assert_eq!(doc["totals"]["recap"][0]["baseCzk"], "2512.50");
    assert_eq!(doc["totals"]["recap"][0]["vatCzk"], "527.63");
    assert_eq!(doc["totals"]["totalCzk"], "3040.13");
    assert_eq!(hits(&counter), 1);

    let body = call(&app, Method::GET, &format!("/api/documents/{first}"), None)
        .await
        .1;
    let second = id(&create_doc(
        &app,
        json!({
            "contactId": body["contactId"], "currency": "EUR", "issueDate": "2026-10-01",
            "taxPointDate": "2026-09-30", "lines": [item("1", "100", "21")]
        }),
    )
    .await);
    assert_eq!(issue(&app, &second).await.0, StatusCode::OK);
    assert_eq!(hits(&counter), 1, "second issue reuses the cached rate");
}

#[tokio::test]
async fn manual_rate_overrides_cnb() {
    let db = TestDb::new().await;
    let (url, counter) = mock_cnb().await;
    let app = router_with_cnb(db.conn.clone(), &url);
    let doc = eur_draft(&app, json!({ "exchangeRate": "24.5" })).await;
    let draft = call(&app, Method::GET, &format!("/api/documents/{doc}"), None)
        .await
        .1;
    assert_eq!(draft["exchangeRateSource"], "manual");
    assert_eq!(
        draft["totals"]["totalCzk"], "2964.50",
        "drafts with a manual rate show CZK"
    );

    let (status, issued) = issue(&app, &doc).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    assert_eq!(issued["exchangeRate"], "24.5");
    assert_eq!(issued["exchangeRateSource"], "manual");
    assert_eq!(issued["exchangeRateDate"], Value::Null);
    assert_eq!(hits(&counter), 0);
}

#[tokio::test]
async fn cnb_down_without_manual_rate_blocks_issue() {
    let db = TestDb::new().await;
    let app = router_with_cnb(db.conn.clone(), &dead_url());
    let doc = eur_draft(&app, json!({})).await;
    let (status, err) = issue(&app, &doc).await;
    assert_eq!(
        (status, err),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "code": "validation", "fields": { "exchangeRate": "required" } })
        )
    );
    let (_, still) = call(&app, Method::GET, &format!("/api/documents/{doc}"), None).await;
    assert_eq!(
        (still["status"].clone(), still["number"].clone()),
        (json!("draft"), Value::Null)
    );

    // A currency ČNB does not list needs a manual rate too.
    let (url, _) = mock_cnb().await;
    let app = router_with_cnb(db.conn.clone(), &url);
    create_bank(&app, "CHF").await;
    let body = call(&app, Method::GET, &format!("/api/documents/{doc}"), None)
        .await
        .1;
    let chf = id(&create_doc(
        &app,
        json!({
            "contactId": body["contactId"], "currency": "CHF", "lines": [item("1", "1", "21")],
            "issueDate": today().checked_sub_days(Days::new(1)).map(|d| d.to_string())
        }),
    )
    .await);
    let (status, err) = issue(&app, &chf).await;
    assert_eq!(
        (status, &err["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "exchangeRate": "required" })
        )
    );
}
