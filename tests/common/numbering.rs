//! Fixtures for the client number sequence tests (`docs/api/numbering.md`).

use axum::Router;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::call;
use super::documents::{create_contact, create_doc, id, issuable, issue};

/// The client pattern the tests give the invoice series.
pub const CLIENT_PATTERN: &str = "{C}-{YYYY}{NNN}";

pub async fn set_client_pattern(
    app: &Router,
    doc_type: &str,
    pattern: &str,
) -> (StatusCode, Value) {
    let uri = format!("/api/settings/number-series/{doc_type}/client-pattern");
    call(
        app,
        Method::PUT,
        &uri,
        Some(json!({ "clientPattern": pattern })),
    )
    .await
}

/// An issuable invoice body (company, bank account, a code-less contact)
/// with the invoice series' client pattern set.
pub async fn client_setup(app: &Router) -> Value {
    let body = issuable(app).await;
    let (status, s) = set_client_pattern(app, "invoice", CLIENT_PATTERN).await;
    assert_eq!(status, StatusCode::OK, "{s}");
    body
}

/// A contact without an IČO (several per test) with `code`.
pub async fn coded_contact(app: &Router, name: &str, code: Option<&str>) -> String {
    create_contact(
        app,
        json!({ "name": name, "ico": null, "numberCode": code }),
    )
    .await
}

/// `body` for `contact`, created and issued; the issued number.
pub async fn issue_for(app: &Router, body: &Value, contact: &str) -> String {
    let mut b = body.clone();
    b["contactId"] = json!(contact);
    let doc = create_doc(app, b).await;
    let (status, issued) = issue(app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    issued["number"].as_str().expect("number").to_string()
}

/// `PUT` a contact with a new number code (other fields as created).
pub async fn set_code(
    app: &Router,
    contact: &str,
    name: &str,
    code: Option<&str>,
) -> (StatusCode, Value) {
    let uri = format!("/api/contacts/{contact}");
    let body = json!({ "name": name, "numberCode": code });
    call(app, Method::PUT, &uri, Some(body)).await
}

/// `PUT` a draft back (the document response is a valid body).
pub async fn put_doc(app: &Router, doc: &Value) -> (StatusCode, Value) {
    let uri = format!("/api/documents/{}", id(doc));
    call(app, Method::PUT, &uri, Some(doc.clone())).await
}
