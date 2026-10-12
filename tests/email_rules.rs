//! Sending rules: validation, imported documents without a PDF, cancelled
//! documents.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{create_issued, get_doc, id};
use common::received::upload;
use common::smtp::{EmailEnv, history, issuable_with_emails, post_email, send_body};
use common::{TestDb, call};
use serde_json::json;

#[tokio::test]
async fn validation_errors() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());
    let doc = create_issued(&app, issuable_with_emails(&app).await).await;
    let many: Vec<String> = (0..11).map(|n| format!("a{n}@example.com")).collect();
    let cases = [
        (json!({ "to": [] }), "to", "required"),
        (json!({ "to": ["not-an-address"] }), "to.0", "invalid"),
        (
            json!({ "cc": ["ok@example.com", "bad"] }),
            "cc.1",
            "invalid",
        ),
        (json!({ "bcc": ["@x"] }), "bcc.0", "invalid"),
        (json!({ "to": many }), "to", "too_long"),
        (json!({ "subject": " " }), "subject", "required"),
        (json!({ "subject": "x".repeat(501) }), "subject", "too_long"),
        (json!({ "subject": "a\nb" }), "subject", "invalid"),
        (json!({ "body": "x".repeat(100_001) }), "body", "too_long"),
    ];
    for (extra, field, reason) in cases {
        let (status, e) = post_email(&app, &id(&doc), send_body(extra.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {e}");
        assert_eq!(
            (&e["code"], &e["fields"][field]),
            (&json!("validation"), &json!(reason)),
            "{extra}"
        );
    }
    assert!(env.smtp.mails().is_empty());
    assert_eq!(history(&app, &id(&doc)).await.1, json!([]));
}

#[tokio::test]
async fn imported_document_without_original_has_no_pdf() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());
    let mut body = issuable_with_emails(&app).await;
    body["imported"] = json!(true);
    body["number"] = json!("FV-2019/0042");
    let doc = create_issued(&app, body).await;
    let doc_id = id(&doc);

    let (_, pre) = call(
        &app,
        Method::GET,
        &format!("/api/documents/{doc_id}/email"),
        None,
    )
    .await;
    assert_eq!(
        pre["attachments"]["pdf"],
        json!({ "available": false, "filename": "FV-2019_0042.pdf" })
    );
    let (status, e) = post_email(&app, &doc_id, send_body(json!({ "attachPdf": true }))).await;
    assert_eq!(
        (status, &e["fields"]["attachPdf"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("invalid"))
    );

    let (status, entry) =
        post_email(&app, &doc_id, send_body(json!({ "attachIsdoc": true }))).await;
    assert_eq!(status, StatusCode::OK, "{entry}");
    assert_eq!(env.smtp.last().attachment_names(), ["FV-2019_0042.isdoc"]);
    assert_eq!(
        env.pdf.mock.count(),
        0,
        "imported documents are never rendered"
    );

    // With its original uploaded, the original is attached.
    let (status, _) = upload(&app, &doc_id, b"%PDF-1.4 original").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, pre) = call(
        &app,
        Method::GET,
        &format!("/api/documents/{doc_id}/email"),
        None,
    )
    .await;
    assert_eq!(pre["attachments"]["pdf"]["available"], true);
    let (status, _) = post_email(&app, &doc_id, send_body(json!({ "attachPdf": true }))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, pdf) = env.smtp.last().attachment("FV-2019_0042.pdf").expect("pdf");
    assert_eq!(pdf, b"%PDF-1.4 original");
}

#[tokio::test]
async fn cancelled_document_can_be_sent() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());
    let doc = create_issued(&app, issuable_with_emails(&app).await).await;
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{}/cancel", id(&doc)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, entry) =
        post_email(&app, &id(&doc), send_body(json!({ "attachPdf": true }))).await;
    assert_eq!(status, StatusCode::OK, "{entry}");
    assert!(get_doc(&app, &id(&doc)).await["sentAt"].is_string());
}

#[tokio::test]
async fn bookkeeping_failure_after_a_delivered_message_is_not_an_error() {
    use sea_orm::ConnectionTrait;
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());
    let doc = create_issued(&app, issuable_with_emails(&app).await).await;
    // The log cannot be written any more.
    db.conn
        .execute_unprepared("DROP TABLE document_emails")
        .await
        .expect("drop the log table");

    let (status, entry) = post_email(&app, &id(&doc), send_body(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{entry}");
    assert_eq!(entry["ok"], true);
    assert!(entry["messageId"].is_string());
    assert_eq!(env.smtp.mails().len(), 1);
    assert!(get_doc(&app, &id(&doc)).await["sentAt"].is_string());

    // A rejected message is still reported as such.
    env.smtp.reject_with("550 no");
    let (status, e) = post_email(&app, &id(&doc), send_body(json!({}))).await;
    assert_eq!(
        (status, &e["code"]),
        (StatusCode::BAD_GATEWAY, &json!("smtp_failed"))
    );
}
