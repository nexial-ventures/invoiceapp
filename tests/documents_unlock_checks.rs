//! Re-issue of an unlocked draft re-checks what relies on it (corrections,
//! deducting invoices); what cannot be unlocked or deleted; an unedited
//! import keeps what was entered.

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use common::documents::{
    create_bank, create_contact, create_issued, get_doc, id, issuable, issue, item, pay,
    post_action, set_company, unlock,
};
use common::{TestDb, call, router};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

async fn put(app: &Router, doc: &Value) -> (StatusCode, Value) {
    let uri = format!("/api/documents/{}", id(doc));
    call(app, Method::PUT, &uri, Some(doc.clone())).await
}

/// Unlock `doc`, apply `edit` to the draft, save it; returns the draft.
async fn unlocked_edit(app: &Router, doc: &Value, edit: impl FnOnce(&mut Value)) -> Value {
    let (status, mut draft) = unlock(app, &id(doc), json!("fix")).await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    edit(&mut draft);
    let (status, saved) = put(app, &draft).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    saved
}

fn conflict(code: &str) -> (StatusCode, Value) {
    (StatusCode::CONFLICT, json!({ "code": code }))
}

async fn credit_note(app: &Router, doc: &Value) -> Value {
    let uri = format!("/api/documents/{}/credit-note", id(doc));
    let body = Some(json!({ "correctionReason": "Sleva" }));
    let (status, credit) = call(app, Method::POST, &uri, body).await;
    assert_eq!(status, StatusCode::CREATED, "{credit}");
    credit
}

#[tokio::test]
async fn issued_credit_notes_cap_the_reissue() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await; // 10 × 100 @ 21 %
    let doc = create_issued(&app, body).await;
    let (status, _) = issue(&app, &id(&credit_note(&app, &doc).await)).await;
    assert_eq!(status, StatusCode::OK);

    let draft = unlocked_edit(&app, &doc, |d| d["lines"] = json!([item("5", "100", "21")])).await;
    assert_eq!(issue(&app, &id(&doc)).await, conflict("exceeds_original"));
    assert_eq!(get_doc(&app, &id(&doc)).await["status"], "draft");

    let mut back = draft.clone();
    back["lines"] = json!([item("10", "100", "21")]);
    assert_eq!(put(&app, &back).await.0, StatusCode::OK);
    let (status, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
}

#[tokio::test]
async fn corrections_bind_contact_currency_and_vat_mode() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    // A draft credit note binds too.
    credit_note(&app, &doc).await;
    let other = create_contact(&app, json!({ "name": "Jiný a.s.", "ico": "87654326" })).await;

    let draft = unlocked_edit(&app, &doc, |d| d["contactId"] = json!(other)).await;
    assert_eq!(issue(&app, &id(&doc)).await, conflict("related_in_use"));

    let mut back = draft.clone();
    back["contactId"] = doc["contactId"].clone();
    assert_eq!(put(&app, &back).await.0, StatusCode::OK);
    assert_eq!(issue(&app, &id(&doc)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn a_deducted_advance_must_keep_what_was_deducted() {
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
    let (status, invoice) = issue(&app, &id(&invoice)).await;
    assert_eq!(status, StatusCode::OK, "{invoice}");

    // Unchanged: re-issues fine.
    let (status, _) = unlock(&app, &id(&proforma), json!("typo")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issue(&app, &id(&proforma)).await.0, StatusCode::OK);

    let eur = create_bank(&app, "EUR").await;
    unlocked_edit(&app, &proforma, |d| {
        d["currency"] = json!("EUR");
        d["exchangeRate"] = json!("25");
        d["bankAccountId"] = json!(eur);
    })
    .await;
    assert_eq!(
        issue(&app, &id(&proforma)).await,
        conflict("advance_settled")
    );
}

#[tokio::test]
async fn a_native_ddpp_never_unlocks() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mut body = issuable(&app).await;
    body["docType"] = json!("proforma");
    body["taxPointDate"] = Value::Null;
    let proforma = create_issued(&app, body).await;
    let (_, payment) = pay(&app, &id(&proforma), json!({ "amount": "1210" })).await;
    let ddpp = payment["advanceDocumentId"]
        .as_str()
        .expect("ddpp")
        .to_string();
    assert_eq!(
        unlock(&app, &ddpp, json!("x")).await,
        conflict("invalid_state")
    );
}

#[tokio::test]
async fn a_correction_whose_original_is_gone_does_not_issue() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    let credit = credit_note(&app, &doc).await;
    db.conn
        .execute_unprepared(&format!(
            "UPDATE documents SET related_document_id = NULL WHERE id = '{}'",
            id(&credit)
        ))
        .await
        .expect("unlink");
    assert_eq!(issue(&app, &id(&credit)).await, conflict("invalid_state"));
}

async fn rename_company(app: &Router, name: &str) {
    let (status, mut company) = call(app, Method::GET, "/api/settings/company", None).await;
    assert_eq!(status, StatusCode::OK);
    company["name"] = json!(name);
    let (status, body) = call(app, Method::PUT, "/api/settings/company", Some(company)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn an_unedited_import_keeps_what_was_entered() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mut body = issuable(&app).await;
    body["imported"] = json!(true);
    body["number"] = json!("FV-2019/0042");
    let doc = create_issued(&app, body).await;
    rename_company(&app, "Nový název s.r.o.").await;

    let (_, draft) = unlock(&app, &id(&doc), json!("look")).await;
    assert_eq!(
        draft["supplier"], doc["supplier"],
        "an import keeps its snapshots"
    );
    let (status, again) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["supplier"]["name"], "Dodavatel s.r.o.");
    assert_eq!(again["totals"], doc["totals"]);

    // Edited: issued like any import, from the current data; its number stays editable.
    unlocked_edit(&app, &doc, |d| d["number"] = json!("FV-2019/0043")).await;
    let (status, edited) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(
        (&edited["number"], &edited["supplier"]["name"]),
        (&json!("FV-2019/0043"), &json!("Nový název s.r.o."))
    );
}

#[tokio::test]
async fn metadata_is_no_edit_of_an_import() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mut body = issuable(&app).await;
    body["imported"] = json!(true);
    body["number"] = json!("FV-2019/0042");
    let doc = create_issued(&app, body).await;
    rename_company(&app, "Nový název s.r.o.").await;

    let (status, _) = unlock(&app, &id(&doc), json!("note only")).await;
    assert_eq!(status, StatusCode::OK);
    let uri = format!("/api/documents/{}/metadata", id(&doc));
    let meta = json!({ "categoryId": null, "customFields": {}, "internalNote": "checked" });
    let (status, saved) = call(&app, Method::PUT, &uri, Some(meta)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, again) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        (&again["supplier"]["name"], &again["internalNote"]),
        (&json!("Dodavatel s.r.o."), &json!("checked"))
    );
    assert_eq!(again["lines"], doc["lines"]);
}
