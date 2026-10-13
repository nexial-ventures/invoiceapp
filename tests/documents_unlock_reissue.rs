//! An unlocked draft: edited and re-issued under its kept (and reserved)
//! number, imported ones without a render, payments carried over, deleted
//! with its payments.

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use common::documents::{
    archive_rel, create_issued, get_doc, id, issuable, issue, item, pay, post_action, set_company,
    unlock,
};
use common::mdcast::PdfEnv;
use common::received::upload;
use common::{TestDb, call, router};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde_json::{Value, json};

/// `sql` with `$1` = the document id → the first column as text.
async fn scalar(db: &DatabaseConnection, sql: &str, doc: &Value) -> Option<String> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            [id(doc).parse::<uuid::Uuid>().expect("uuid").into()],
        ))
        .await
        .expect("query")
        .expect("a row");
    row.try_get_by_index(0).expect("text column")
}

async fn put(app: &Router, doc: &Value) -> Value {
    let uri = format!("/api/documents/{}", id(doc));
    let (status, saved) = call(app, Method::PUT, &uri, Some(doc.clone())).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    saved
}

#[tokio::test]
async fn reissue_keeps_the_number_after_a_date_change() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    let (_, mut draft) = unlock(&app, &id(&doc), json!("Wrong year")).await;

    // The type stays: an unlocked draft is edited like any other.
    let mut retyped = draft.clone();
    retyped["docType"] = json!("proforma");
    retyped["taxPointDate"] = Value::Null;
    let uri = format!("/api/documents/{}", id(&draft));
    let (status, err) = call(&app, Method::PUT, &uri, Some(retyped)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["fields"]["docType"], "invalid");

    draft["issueDate"] = json!("2027-01-04");
    draft["taxPointDate"] = json!("2027-01-04");
    draft["dueDate"] = json!("2027-01-18");
    draft["lines"] = json!([item("2", "300", "21")]);
    let saved = put(&app, &draft).await;
    assert_eq!(
        (&saved["number"], &saved["status"]),
        (&json!("20260001"), &json!("draft"))
    );
    // A native number may be sent back unchanged, never altered.
    let mut other = saved.clone();
    other["number"] = json!("X-1");
    let uri = format!("/api/documents/{}", id(&saved));
    let (status, err) = call(&app, Method::PUT, &uri, Some(other)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["fields"]["number"], "invalid");

    let renders = env.mock.count();
    let (status, reissued) = issue(&app, &id(&saved)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
    assert_eq!(
        (
            &reissued["status"],
            &reissued["docType"],
            &reissued["number"]
        ),
        (&json!("issued"), &json!("invoice"), &json!("20260001"))
    );
    assert_eq!(reissued["totals"]["payable"], "726.00");
    assert_eq!(reissued["supplier"]["name"], "Dodavatel s.r.o.");
    assert_eq!(reissued["unlockReason"], "Wrong year");
    let year = scalar(
        &db.conn,
        "SELECT number_year::text FROM documents WHERE id = $1",
        &doc,
    )
    .await;
    assert_eq!(year.as_deref(), Some("2026"), "the number keeps its year");
    // Rendered and archived again.
    assert_eq!(env.mock.count(), renders + 1);
    assert_eq!(env.mock.last().data["number"], "20260001");
    let archive = env
        .storage
        .path()
        .join(db.key(&archive_rel(&db.conn, &reissued).await));
    assert!(archive.exists());
}

#[tokio::test]
async fn the_kept_number_stays_reserved() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let first = create_issued(&app, body.clone()).await;
    let (status, _) = unlock(&app, &id(&first), json!("fix")).await;
    assert_eq!(status, StatusCode::OK);

    // The counter guard still counts the unlocked draft's number.
    let uri = "/api/settings/number-series/invoice/counters/2026";
    let (status, err) = call(&app, Method::PUT, uri, Some(json!({ "lastNumber": 0 }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["fields"]["lastNumber"], "below_issued");

    let second = create_issued(&app, body).await;
    assert_eq!(second["number"], "20260002");
    let (status, again) = issue(&app, &id(&first)).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["number"], "20260001");
    let (_, all) = call(&app, Method::GET, "/api/settings/number-series", None).await;
    let series = all
        .as_array()
        .and_then(|a| a.iter().find(|s| s["docType"] == "invoice"))
        .expect("invoice series");
    assert_eq!(series["counters"][0]["lastNumber"], 2, "{series}");
}

#[tokio::test]
async fn imported_document_unlocks_and_reissues_without_a_render() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let mut body = issuable(&app).await;
    body["imported"] = json!(true);
    body["number"] = json!("FV-2019/0042");
    let doc = create_issued(&app, body).await;
    let (status, _) = upload(&app, &id(&doc), b"%PDF-1.4 original").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let original_sql = "SELECT original_path FROM documents WHERE id = $1";
    let rel = scalar(&db.conn, original_sql, &doc)
        .await
        .expect("original");
    let original = env.storage.path().join(db.key(&rel));

    let (status, mut draft) = unlock(&app, &id(&doc), json!("Wrong order ref")).await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    assert_eq!(
        (&draft["imported"], &draft["number"]),
        (&json!(true), &json!("FV-2019/0042"))
    );
    assert!(draft["original"].is_object(), "the uploaded original stays");
    assert!(original.exists());

    draft["orderRef"] = json!("PO-7");
    put(&app, &draft).await;
    let (status, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
    assert_eq!(
        (
            &reissued["imported"],
            &reissued["number"],
            &reissued["orderRef"]
        ),
        (&json!(true), &json!("FV-2019/0042"), &json!("PO-7"))
    );
    assert_eq!((&reissued["pdf"], env.mock.count()), (&Value::Null, 0));
    assert_eq!(scalar(&db.conn, original_sql, &doc).await, Some(rel));
    assert!(original.exists());
}

#[tokio::test]
async fn payments_survive_the_unlock() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await; // payable 1210
    let doc = create_issued(&app, body).await;
    let (status, payment) = pay(&app, &id(&doc), json!({ "amount": "500" })).await;
    assert_eq!(status, StatusCode::CREATED, "{payment}");

    let (_, draft) = unlock(&app, &id(&doc), json!("fix")).await;
    assert_eq!(
        (&draft["paid"], &draft["paymentState"]),
        (&json!("500.00"), &Value::Null)
    );
    let uri = format!("/api/documents/{}/payments", id(&doc));
    let (_, listed) = call(&app, Method::GET, &uri, None).await;
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
    let conflict = (StatusCode::CONFLICT, json!({ "code": "invalid_state" }));
    assert_eq!(
        pay(&app, &id(&doc), json!({ "amount": "1" })).await,
        conflict
    );
    let one = format!("{uri}/{}", payment["id"].as_str().expect("payment id"));
    assert_eq!(call(&app, Method::DELETE, &one, None).await, conflict);

    let (status, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
    assert_eq!(
        (&reissued["paid"], &reissued["paymentState"]),
        (&json!("500.00"), &json!("partial"))
    );
}

#[tokio::test]
async fn deleting_an_unlocked_draft_cascades_and_unlinks() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    let (status, _) = pay(&app, &id(&doc), json!({ "amount": "100" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let uri = format!("/api/documents/{}/credit-note", id(&doc));
    let (status, credit) = call(
        &app,
        Method::POST,
        &uri,
        Some(json!({ "correctionReason": "Sleva" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credit}");
    let (status, credit) = issue(&app, &id(&credit)).await;
    assert_eq!(status, StatusCode::OK, "{credit}");

    let (status, _) = unlock(&app, &id(&doc), json!("cancel by delete")).await;
    assert_eq!(status, StatusCode::OK);
    let uri = format!("/api/documents/{}", id(&doc));
    // A live credit note still corrects it.
    let (status, err) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(
        (status, err),
        (StatusCode::CONFLICT, json!({ "code": "related_in_use" }))
    );
    let (status, _) = post_action(&app, &id(&credit), "cancel").await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let payments = "SELECT count(*)::text FROM payments WHERE document_id = $1";
    assert_eq!(scalar(&db.conn, payments, &doc).await.as_deref(), Some("0"));
    let credit = get_doc(&app, &id(&credit)).await;
    assert_eq!(
        (&credit["relatedDocumentId"], &credit["status"]),
        (&Value::Null, &json!("cancelled"))
    );
}

#[tokio::test]
async fn a_deducted_advance_cannot_be_deleted() {
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

    let (status, _) = unlock(&app, &id(&proforma), json!("fix")).await;
    assert_eq!(status, StatusCode::OK);
    let uri = format!("/api/documents/{}", id(&proforma));
    let (status, err) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(
        (status, err),
        (StatusCode::CONFLICT, json!({ "code": "advance_in_use" }))
    );
    assert_eq!(
        get_doc(&app, &id(&invoice)).await["lines"][1]["kind"],
        "advance"
    );

    // Without the deducting draft it goes, payments and all.
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/documents/{}", id(&invoice)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
