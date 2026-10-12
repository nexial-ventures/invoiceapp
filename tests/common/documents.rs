//! Fixtures for the document tests: company, contacts, bank accounts, lines,
//! and a mock ČNB server.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};

use super::call;

pub async fn set_company(app: &Router, vat_payer: bool) {
    let (status, body) = call(
        app,
        Method::PUT,
        "/api/settings/company",
        Some(json!({
            "name": "Dodavatel s.r.o.",
            "ico": "44444443",
            "dic": "CZ44444443",
            "vatPayer": vat_payer,
            "street": "Hlavní 1",
            "city": "Praha",
            "zip": "11000",
            "country": "CZ",
            "registration": "C 123 vedená u MS v Praze",
            "defaultDueDays": 14,
            "defaultLocale": "cs"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A contact; `extra` fields override the defaults.
pub async fn create_contact(app: &Router, extra: Value) -> String {
    let mut body = json!({ "name": "Odběratel a.s.", "ico": "12345679", "city": "Brno" });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    let (status, c) = call(app, Method::POST, "/api/contacts", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{c}");
    c["id"].as_str().expect("contact id").to_string()
}

pub async fn create_bank(app: &Router, currency: &str) -> String {
    let (status, b) = call(
        app,
        Method::POST,
        "/api/settings/bank-accounts",
        Some(json!({ "currency": currency, "accountNumber": "19-2000145399/0800", "bic": "GIBACZPX" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{b}");
    b["id"].as_str().expect("bank id").to_string()
}

pub fn item(qty: &str, price: &str, rate: &str) -> Value {
    json!({ "kind": "item", "description": "Práce", "quantity": qty, "unit": "h",
            "unitPrice": price, "vatRate": rate })
}

/// POST a document; asserts 201.
pub async fn create_doc(app: &Router, body: Value) -> Value {
    let (status, doc) = call(app, Method::POST, "/api/documents", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{doc}");
    doc
}

pub fn id(doc: &Value) -> String {
    doc["id"].as_str().expect("document id").to_string()
}

/// The archive key (within its space) of an issued document, as stored in
/// `pdf_path` (`documents/{year}/{id}-{sha8}-{nonce}.pdf`).
pub async fn archive_rel(db: &sea_orm::DatabaseConnection, doc: &Value) -> String {
    use sea_orm::{ConnectionTrait, Statement};
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT pdf_path FROM documents WHERE id = $1",
            [id(doc).parse::<uuid::Uuid>().expect("uuid").into()],
        ))
        .await
        .expect("read pdf_path")
        .expect("document row");
    let rel: String = row.try_get_by_index(0).expect("archived document");
    let sha = doc["pdf"]["sha256"].as_str().expect("sha256");
    assert!(rel.contains(&format!("-{}-", &sha[..8])), "{rel}");
    rel
}

pub async fn issue(app: &Router, doc_id: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        &format!("/api/documents/{doc_id}/issue"),
        None,
    )
    .await
}

/// A payer company, a contact and a CZK account → an issuable draft body.
pub async fn issuable(app: &Router) -> Value {
    set_company(app, true).await;
    let contact = create_contact(app, json!({})).await;
    create_bank(app, "CZK").await;
    json!({ "contactId": contact, "issueDate": "2026-10-01", "lines": [item("10", "100", "21")] })
}

#[derive(Clone)]
struct Mock {
    hits: Arc<AtomicUsize>,
    template: String,
    /// Publication date to answer with; `None` echoes the requested date.
    published: Option<String>,
}

async fn mock_rates(
    State(m): State<Mock>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    m.hits.fetch_add(1, Ordering::SeqCst);
    match m.published.as_ref().or(q.get("date")) {
        Some(d) => (StatusCode::OK, m.template.replace("{DATE}", d)).into_response(),
        None => (StatusCode::BAD_REQUEST, "date required").into_response(),
    }
}

/// Mock ČNB echoing the requested date as the publication date; returns its
/// `denni_kurz.txt` URL and a hit counter.
pub async fn mock_cnb() -> (String, Arc<AtomicUsize>) {
    mock_cnb_published(None).await
}

/// Mock ČNB that always answers with the list published on `published`
/// (`DD.MM.YYYY`), like ČNB before the day's list is out.
pub async fn mock_cnb_published(published: Option<&str>) -> (String, Arc<AtomicUsize>) {
    let path = format!(
        "{}/tests/fixtures/cnb/denni_kurz.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let template = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/denni_kurz.txt", get(mock_rates))
        .with_state(Mock {
            hits: hits.clone(),
            template,
            published: published.map(str::to_string),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock ČNB");
    let addr = listener.local_addr().expect("mock address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock ČNB server");
    });
    (format!("http://{addr}/denni_kurz.txt"), hits)
}

/// A URL nothing listens on.
pub fn dead_url() -> String {
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("free port");
    format!("http://{addr}/denni_kurz.txt")
}

pub fn hits(counter: &Arc<AtomicUsize>) -> usize {
    counter.load(Ordering::SeqCst)
}

/// POST + issue; asserts both succeed and returns the issued document.
pub async fn create_issued(app: &Router, body: Value) -> Value {
    let doc = create_doc(app, body).await;
    let (status, issued) = issue(app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    issued
}

pub async fn get_doc(app: &Router, doc_id: &str) -> Value {
    let (status, doc) = call(app, Method::GET, &format!("/api/documents/{doc_id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    doc
}

/// POST a payment with `body` (date defaults to 2026-10-05).
pub async fn pay(app: &Router, doc_id: &str, body: Value) -> (StatusCode, Value) {
    let mut b = json!({ "date": "2026-10-05" });
    if let (Some(o), Some(extra)) = (b.as_object_mut(), body.as_object()) {
        o.extend(extra.clone());
    }
    call(
        app,
        Method::POST,
        &format!("/api/documents/{doc_id}/payments"),
        Some(b),
    )
    .await
}

pub async fn post_action(app: &Router, doc_id: &str, action: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        &format!("/api/documents/{doc_id}/{action}"),
        None,
    )
    .await
}
