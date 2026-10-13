//! Space delete: owner + session only, slug + password, rows and files gone (spaces.md, 4a).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, member, on, seed_space};
use common::mdcast::PdfEnv;
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb};
use invoiceapp::space::Role;
use serde_json::json;

#[tokio::test]
async fn space_delete_removes_rows_files_and_frees_the_slug() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let (app, pdf) = env.router_and_pdf(db.conn.clone());
    let (other, _) = seed_space(&db.conn, "beta", "beta@example.com", None).await;
    // Data and files in acme (a custom logo the issue render caches); one
    // file in beta that must survive.
    env.storage
        .storage
        .put(
            &db.key("design/logo.png"),
            bytes::Bytes::from_static(b"png"),
        )
        .await
        .expect("put logo");
    let body = common::documents::issuable(&app).await;
    let doc = common::documents::create_doc(&app, body).await;
    let (s, _) = common::documents::issue(&app, &common::documents::id(&doc)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(pdf.design_cache().len(), 1, "the logo is cached");
    assert!(
        !env.storage
            .keys(&db.space.storage_prefix())
            .await
            .is_empty()
    );
    let beta_key = format!("{}/documents/keep.pdf", other.storage_prefix());
    env.storage
        .storage
        .put(&beta_key, bytes::Bytes::from_static(b"%PDF-"))
        .await
        .expect("put");

    let cookie = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let del = |slug: &str, password: &str| json!({ "slug": slug, "password": password });
    let (s, j) = on(
        &app,
        Method::DELETE,
        TEST_HOST,
        "/api/space",
        As::Bearer(TEST_TOKEN),
        Some(del("acme", OWNER_PASSWORD)),
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "forbidden" })),
        "a token, even the owner's"
    );
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let (s, _) = on(
        &app,
        Method::DELETE,
        TEST_HOST,
        "/api/space",
        As::Bearer(&admin),
        Some(del("acme", OWNER_PASSWORD)),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, j) = on(
        &app,
        Method::DELETE,
        TEST_HOST,
        "/api/space",
        As::Cookie(&cookie),
        Some(del("beta", "wrong password!")),
    )
    .await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "slug": "mismatch", "password": "invalid" })
        )
    );

    let (s, _) = on(
        &app,
        Method::DELETE,
        TEST_HOST,
        "/api/space",
        As::Cookie(&cookie),
        Some(del("acme", OWNER_PASSWORD)),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/context",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "the host is gone");
    assert!(
        pdf.design_cache().is_empty(),
        "its cached design is dropped"
    );
    // The files go in a detached task after the commit.
    let mut gone = false;
    for _ in 0..500 {
        if env
            .storage
            .keys(&db.space.storage_prefix())
            .await
            .is_empty()
        {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(gone, "files removed");
    assert!(
        env.storage.bytes(&beta_key).await.is_some(),
        "other spaces untouched"
    );
    use sea_orm::ConnectionTrait;
    for table in [
        "documents",
        "contacts",
        "company",
        "vat_rates",
        "number_series",
        "bank_accounts",
        "accounting_settings",
        "space_members",
        "api_tokens",
        "sessions",
    ] {
        let row = db
            .conn
            .query_one(sea_orm::Statement::from_string(
                db.conn.get_database_backend(),
                format!(
                    "SELECT count(*) AS n FROM {table} WHERE space_id = '{}'",
                    db.space
                ),
            ))
            .await
            .expect("count")
            .and_then(|r| r.try_get::<i64>("", "n").ok());
        assert_eq!(row, Some(0), "{table}");
    }
    let (s, _) = on(
        &app,
        Method::GET,
        "beta.localhost:3000",
        "/api/context",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // The slug is free again (space creation needs registration on).
    let app = common::router_open(db.conn.clone());
    let base = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("still a user");
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&base),
        Some(json!({ "slug": "acme", "name": "Again" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn space_delete_with_advances_and_corrections() {
    use common::documents::{create_issued, issuable, pay, post_action};
    let db = TestDb::new().await;
    let app = common::router(db.conn.clone());
    // Proforma → payment → DDPP → settled final invoice deducting it (an
    // `advance` line referencing the DDPP) → a credit note of the invoice.
    let mut body = issuable(&app).await;
    body["docType"] = json!("proforma");
    let p = create_issued(&app, body).await;
    let (s, payment) = pay(
        &app,
        &common::documents::id(&p),
        json!({ "amount": "1210" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{payment}");
    assert!(
        payment["advanceDocumentId"].is_string(),
        "a DDPP was issued"
    );
    let (s, inv) = post_action(&app, &common::documents::id(&p), "settle").await;
    assert_eq!(s, StatusCode::CREATED, "{inv}");
    let (s, inv) = post_action(&app, &common::documents::id(&inv), "issue").await;
    assert_eq!(s, StatusCode::OK, "{inv}");
    assert_eq!(inv["lines"][1]["kind"], "advance");
    let (s, credit) = post_action(&app, &common::documents::id(&inv), "credit-note").await;
    assert_eq!(s, StatusCode::CREATED, "{credit}");
    // An imported document referencing an original, and a category in use.
    let (s, cat) = common::call(
        &app,
        Method::POST,
        "/api/settings/categories",
        Some(json!({ "name": "Prodej", "kind": "income" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{cat}");
    let meta = json!({ "categoryId": cat["id"], "customFields": {} });
    let uri = format!("/api/documents/{}/metadata", common::documents::id(&inv));
    let (s, _) = common::call(&app, Method::PUT, &uri, Some(meta)).await;
    assert_eq!(s, StatusCode::OK);

    let cookie = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let body = json!({ "slug": "acme", "password": OWNER_PASSWORD });
    let (s, j) = on(
        &app,
        Method::DELETE,
        TEST_HOST,
        "/api/space",
        As::Cookie(&cookie),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    use sea_orm::ConnectionTrait;
    let left = db
        .conn
        .query_one(sea_orm::Statement::from_string(
            db.conn.get_database_backend(),
            "SELECT (SELECT count(*) FROM documents) + (SELECT count(*) FROM document_lines) \
             + (SELECT count(*) FROM payments) + (SELECT count(*) FROM categories) AS n",
        ))
        .await
        .expect("count")
        .and_then(|r| r.try_get::<i64>("", "n").ok());
    assert_eq!(left, Some(0));
}
