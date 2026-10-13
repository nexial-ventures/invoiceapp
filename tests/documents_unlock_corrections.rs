//! Unlocked corrections and deducted advances: a debit note raises its
//! original's credit cap and a DDPP correction changes a deducted DDPP, so
//! their re-issue and delete keep the guards of a cancel.

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use common::documents::{
    create_bank, create_issued, id, issuable, issue, item, pay, post_action, set_company, unlock,
};
use common::{TestDb, call, router};
use serde_json::{Value, json};

fn conflict(code: &str) -> (StatusCode, Value) {
    (StatusCode::CONFLICT, json!({ "code": code }))
}

/// `POST …/{action}` with a correction reason, the lines replaced, issued.
async fn issued_note(app: &Router, original: &str, action: &str, lines: Value) -> Value {
    let uri = format!("/api/documents/{original}/{action}");
    let body = Some(json!({ "correctionReason": "Oprava" }));
    let (status, mut note) = call(app, Method::POST, &uri, body).await;
    assert_eq!(status, StatusCode::CREATED, "{note}");
    note["lines"] = lines;
    save(app, &note).await;
    let (status, note) = issue(app, &id(&note)).await;
    assert_eq!(status, StatusCode::OK, "{note}");
    note
}

async fn save(app: &Router, doc: &Value) -> Value {
    let uri = format!("/api/documents/{}", id(doc));
    let (status, saved) = call(app, Method::PUT, &uri, Some(doc.clone())).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    saved
}

async fn delete(app: &Router, doc: &Value) -> (StatusCode, Value) {
    let uri = format!("/api/documents/{}", id(doc));
    call(app, Method::DELETE, &uri, None).await
}

#[tokio::test]
async fn a_debit_note_raises_and_holds_the_cap() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await; // base 1000 @ 21 %
    let invoice = create_issued(&app, body).await;
    let debit = issued_note(
        &app,
        &id(&invoice),
        "debit-note",
        json!([item("1", "500", "21")]),
    )
    .await;
    issued_note(
        &app,
        &id(&invoice),
        "credit-note",
        json!([item("14", "100", "21")]),
    )
    .await;

    // The invoice re-issues unchanged: 1400 credited ≤ 1000 + the 500 debit.
    let (status, _) = unlock(&app, &id(&invoice), json!("check")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, again) = issue(&app, &id(&invoice)).await;
    assert_eq!(status, StatusCode::OK, "{again}");

    // The debit note lowered to 300 would leave 1400 credited over a 1300 cap.
    let (_, mut draft) = unlock(&app, &id(&debit), json!("too high")).await;
    draft["lines"] = json!([item("1", "300", "21")]);
    let draft = save(&app, &draft).await;
    assert_eq!(issue(&app, &id(&debit)).await, conflict("exceeds_original"));
    // Deleting it is a cancel: the same cap.
    assert_eq!(delete(&app, &debit).await, conflict("exceeds_original"));

    let mut back = draft.clone();
    back["lines"] = json!([item("1", "450", "21")]);
    save(&app, &back).await;
    let (status, reissued) = issue(&app, &id(&debit)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
}

#[tokio::test]
async fn a_correction_of_a_deducted_ddpp_stays_put() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mut body = issuable(&app).await;
    body["docType"] = json!("proforma");
    let proforma = create_issued(&app, body).await;
    let (_, payment) = pay(&app, &id(&proforma), json!({ "amount": "1210" })).await;
    let ddpp = payment["advanceDocumentId"]
        .as_str()
        .expect("DDPP")
        .to_string();
    let correction = issued_note(&app, &ddpp, "credit-note", json!([item("1", "100", "21")])).await;
    let (status, invoice) = post_action(&app, &id(&proforma), "settle").await;
    assert_eq!(status, StatusCode::CREATED, "{invoice}");

    // Unlocking is free; bringing it back or deleting it would change what the draft invoice deducts.
    let (status, _) = unlock(&app, &id(&correction), json!("typo")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        issue(&app, &id(&correction)).await,
        conflict("advance_in_use")
    );
    assert_eq!(delete(&app, &correction).await, conflict("advance_in_use"));
}

#[tokio::test]
async fn a_draft_deducting_the_advance_blocks_a_changed_reissue() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mut body = issuable(&app).await;
    set_company(&app, false).await;
    body["docType"] = json!("proforma");
    body["lines"] = json!([item("1", "1000", "0")]);
    let proforma = create_issued(&app, body).await;
    let (status, _) = pay(&app, &id(&proforma), json!({ "amount": "400" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, invoice) = post_action(&app, &id(&proforma), "settle").await;
    assert_eq!(status, StatusCode::CREATED, "{invoice}");

    let eur = create_bank(&app, "EUR").await;
    let (_, mut draft) = unlock(&app, &id(&proforma), json!("currency")).await;
    draft["currency"] = json!("EUR");
    draft["exchangeRate"] = json!("25");
    draft["bankAccountId"] = json!(eur);
    save(&app, &draft).await;
    assert_eq!(
        issue(&app, &id(&proforma)).await,
        conflict("advance_in_use")
    );
}
