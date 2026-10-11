//! Two spaces with identical data never see each other's records (spaces.md,
//! 4a): lists, reads, writes, number allocation, references, exports, MCP,
//! PDFs and storage keys.

mod common;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use common::auth::{As, on, seed_space};
use common::documents::{create_doc, id, issuable, issue, item};
use common::mcp::tool_as;
use common::mdcast::PdfEnv;
use common::members::{B_HOST, B_TOKEN};
use common::{TEST_HOST, TEST_TOKEN, TestDb, call};
use serde_json::{Value, json};

/// `app` as seen by space beta: every request goes to beta's host with
/// beta's token, so the shared helpers (which use the default host and
/// token) act in beta.
fn as_beta(app: &Router) -> Router {
    app.clone().layer(axum::middleware::map_request(
        |mut req: Request| async move {
            let h = req.headers_mut();
            h.insert(header::HOST, HeaderValue::from_static(B_HOST));
            if h.contains_key(header::AUTHORIZATION) {
                let bearer = HeaderValue::from_str(&format!("Bearer {B_TOKEN}")).expect("header");
                h.insert(header::AUTHORIZATION, bearer);
            }
            req
        },
    ))
}

/// The same data in a space: company, contact, bank, an issued invoice.
async fn fill(app: &Router) -> (Value, Value) {
    let body = issuable(app).await;
    let draft = create_doc(app, body.clone()).await;
    let (s, doc) = issue(app, &id(&draft)).await;
    assert_eq!(s, StatusCode::OK, "{doc}");
    (body, doc)
}

#[tokio::test]
async fn two_spaces_with_identical_data_stay_apart() {
    let db = TestDb::new().await;
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    let env = PdfEnv::new();
    let a = env.router(db.conn.clone());
    let b = as_beta(&a);
    let (a_body, a_doc) = fill(&a).await;
    let (_, b_doc) = fill(&b).await;

    // Numbering is per space: both got the first number of the year.
    assert_eq!(a_doc["number"], b_doc["number"]);
    assert_ne!(a_doc["id"], b_doc["id"]);
    for (app, own) in [(&a, &a_doc), (&b, &b_doc)] {
        let (_, list) = call(app, Method::GET, "/api/documents?direction=issued", None).await;
        assert_eq!(list["total"], 1);
        assert_eq!(list["items"][0]["id"], own["id"]);
        let (_, contacts) = call(app, Method::GET, "/api/contacts", None).await;
        assert_eq!(contacts["total"], 1);
    }

    // Another space's id in a URL is not found — read, write, delete, actions.
    let foreign = id(&a_doc);
    for (m, uri, body) in [
        (Method::GET, format!("/api/documents/{foreign}"), None),
        (
            Method::PUT,
            format!("/api/documents/{foreign}"),
            Some(a_body.clone()),
        ),
        (Method::DELETE, format!("/api/documents/{foreign}"), None),
        (Method::GET, format!("/api/documents/{foreign}/pdf"), None),
        (
            Method::GET,
            format!("/api/documents/{foreign}/payments"),
            None,
        ),
        (
            Method::POST,
            format!("/api/documents/{foreign}/payments"),
            Some(json!({ "date": "2026-10-02", "amount": "1" })),
        ),
        (
            Method::POST,
            format!("/api/documents/{foreign}/cancel"),
            None,
        ),
        (
            Method::POST,
            format!("/api/documents/{foreign}/credit-note"),
            None,
        ),
        (
            Method::PUT,
            format!("/api/documents/{foreign}/internal-note"),
            Some(json!({ "internalNote": "x" })),
        ),
        (Method::GET, format!("/api/documents/{foreign}/isdoc"), None),
        (
            Method::GET,
            format!("/api/documents/{foreign}/emails"),
            None,
        ),
        (
            Method::GET,
            format!(
                "/api/contacts/{}",
                a_body["contactId"].as_str().expect("contact")
            ),
            None,
        ),
        (
            Method::DELETE,
            format!("/api/settings/bank-accounts/{}", bank_of(&a).await),
            None,
        ),
    ] {
        let (s, j) = call(&b, m.clone(), &uri, body).await;
        assert_eq!(
            (s, j),
            (StatusCode::NOT_FOUND, json!({ "code": "not_found" })),
            "{m} {uri}"
        );
    }
    let (_, still) = call(&a, Method::GET, &format!("/api/documents/{foreign}"), None).await;
    assert_eq!(still["internalNote"], Value::Null, "untouched");

    // A foreign reference behaves like a non-existent one.
    let random = uuid::Uuid::new_v4().to_string();
    for field in ["contactId", "bankAccountId"] {
        let foreign_ref = a_body[field]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| bank_sync(&a_doc));
        let mut with_foreign = a_body.clone();
        with_foreign[field] = json!(foreign_ref);
        let mut with_random = a_body.clone();
        with_random[field] = json!(random);
        let (s1, j1) = call(&b, Method::POST, "/api/documents", Some(with_foreign)).await;
        let (s2, j2) = call(&b, Method::POST, "/api/documents", Some(with_random)).await;
        assert_eq!((s1, &j1), (s2, &j2), "{field}");
        assert_eq!(s1, StatusCode::UNPROCESSABLE_ENTITY, "{field}: {j1}");
    }
    let (s1, j1) = call(
        &b,
        Method::POST,
        "/api/documents/compute",
        Some(json!({ "documentId": foreign, "lines": [] })),
    )
    .await;
    let (s2, j2) = call(
        &b,
        Method::POST,
        "/api/documents/compute",
        Some(json!({ "documentId": random, "lines": [] })),
    )
    .await;
    assert_eq!((s1, j1), (s2, j2), "compute documentId");
    let related = |rel: &str| {
        json!({ "imported": true, "number": "X-1", "docType": "credit_note",
        "relatedDocumentId": rel, "lines": [item("1", "1", "21")] })
    };
    let (s1, j1) = call(&b, Method::POST, "/api/documents", Some(related(&foreign))).await;
    let (s2, j2) = call(&b, Method::POST, "/api/documents", Some(related(&random))).await;
    assert_eq!((s1, j1), (s2, j2), "relatedDocumentId");

    // Exports carry only the space's own documents.
    let csv = common::csv_export::export(&b, "/api/export/csv?direction=issued").await;
    assert_eq!(csv.rows.len(), 1, "only beta's document");
    let csv = common::csv_export::export(&a, "/api/export/csv?direction=issued").await;
    assert_eq!(csv.rows.len(), 1, "only acme's document");

    // Archives live under each space's prefix.
    let a_key = db.key(&format!("documents/2026/{}.pdf", id(&a_doc)));
    let b_key = format!(
        "{}/documents/2026/{}.pdf",
        beta.storage_prefix(),
        id(&b_doc)
    );
    assert!(env.storage.bytes(&a_key).await.is_some(), "{a_key}");
    assert!(env.storage.bytes(&b_key).await.is_some(), "{b_key}");
    assert!(
        env.storage
            .keys(&db.key("documents"))
            .await
            .iter()
            .all(|k| !k.contains(&id(&b_doc)))
    );

    // MCP: a token never works on another space's host; tools see one space.
    let (s, _) = on(
        &a,
        Method::POST,
        B_HOST,
        "/api/mcp",
        As::Bearer(TEST_TOKEN),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let listed = tool_as(
        &a,
        B_HOST,
        B_TOKEN,
        "list_documents",
        json!({ "direction": "issued" }),
    )
    .await;
    assert_eq!(listed["structuredContent"]["total"], 1);
    assert_eq!(listed["structuredContent"]["items"][0]["id"], b_doc["id"]);
    let got = tool_as(
        &a,
        B_HOST,
        B_TOKEN,
        "get_document",
        json!({ "id": foreign }),
    )
    .await;
    assert_eq!(got["structuredContent"], json!({ "code": "not_found" }));
    let got = tool_as(
        &a,
        TEST_HOST,
        TEST_TOKEN,
        "get_document",
        json!({ "id": foreign }),
    )
    .await;
    assert_eq!(got["isError"], false);
}

#[tokio::test]
async fn settings_and_catalog_are_per_space() {
    let db = TestDb::new().await;
    seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    let a = common::router(db.conn.clone());
    let b = as_beta(&a);

    // The same unique values in both spaces (per-space unique indexes).
    for app in [&a, &b] {
        let (s, j) = call(
            app,
            Method::POST,
            "/api/settings/vat-rates",
            Some(json!({ "rate": "15", "label": "x" })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{j}");
        let (s, j) = call(
            app,
            Method::POST,
            "/api/settings/categories",
            Some(json!({ "name": "Nájem", "kind": "expense" })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{j}");
        let (s, j) = call(
            app,
            Method::POST,
            "/api/contacts",
            Some(json!({ "name": "C", "ico": "87654326" })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{j}");
        let (s, j) = call(app, Method::POST, "/api/settings/custom-fields",
            Some(json!({ "key": "zakazka", "label": "Zakázka", "type": "text", "appliesTo": "both" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{j}");
    }
    let (_, ra) = call(&a, Method::GET, "/api/settings/vat-rates", None).await;
    assert_eq!(ra.as_array().map(Vec::len), Some(4));
    let (s, _) = call(
        &a,
        Method::PUT,
        "/api/settings/number-series/invoice",
        Some(json!({ "pattern": "A{YYYY}{NNNN}" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, sb) = call(&b, Method::GET, "/api/settings/number-series", None).await;
    assert_eq!(
        sb[0]["pattern"], "{YYYY}{NNNN}",
        "the other space's series unchanged"
    );

    // A foreign catalog item in a group behaves like an unknown one.
    let (_, item_a) = call(
        &a,
        Method::POST,
        "/api/catalog/items",
        Some(json!({ "name": "I", "unitPrice": "1", "currency": "CZK", "vatRate": "21" })),
    )
    .await;
    let group =
        |item: &Value| json!({ "name": "G", "members": [{ "itemId": item, "quantity": "1" }] });
    let (s1, j1) = call(
        &b,
        Method::POST,
        "/api/catalog/groups",
        Some(group(&item_a["id"])),
    )
    .await;
    let (s2, j2) = call(
        &b,
        Method::POST,
        "/api/catalog/groups",
        Some(group(&json!(uuid::Uuid::new_v4()))),
    )
    .await;
    assert_eq!((s1, j1), (s2, j2));
    let (s, _) = call(
        &b,
        Method::GET,
        &format!("/api/catalog/items/{}", item_a["id"].as_str().expect("id")),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // A foreign category in document metadata behaves like an unknown one.
    let (_, cats) = call(&a, Method::GET, "/api/settings/categories", None).await;
    let cat_a = cats[0]["id"].clone();
    let (_, doc_b) = call(
        &b,
        Method::POST,
        "/api/documents",
        Some(json!({ "lines": [] })),
    )
    .await;
    let meta = |c: &Value| json!({ "categoryId": c, "customFields": {} });
    let uri = format!("/api/documents/{}/metadata", id(&doc_b));
    let (s1, j1) = call(&b, Method::PUT, &uri, Some(meta(&cat_a))).await;
    let (s2, j2) = call(
        &b,
        Method::PUT,
        &uri,
        Some(meta(&json!(uuid::Uuid::new_v4()))),
    )
    .await;
    assert_eq!((s1, &j1), (s2, &j2));
    assert_eq!(s1, StatusCode::UNPROCESSABLE_ENTITY, "{j1}");
}

async fn bank_of(app: &Router) -> String {
    let (_, banks) = call(app, Method::GET, "/api/settings/bank-accounts", None).await;
    banks[0]["id"].as_str().expect("bank").to_string()
}

fn bank_sync(doc: &Value) -> String {
    doc["bankAccountId"]
        .as_str()
        .expect("bank account")
        .to_string()
}
