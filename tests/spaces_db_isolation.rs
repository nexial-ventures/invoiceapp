//! Tenant isolation enforced by the database itself
//! (`m20261023_000001_tenant_fks`): a document can only reference a contact,
//! bank account, category or document of its own space — even a raw SQL
//! write that bypasses every repo is refused.

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use common::auth::seed_space;
use common::documents::{create_bank, create_contact, create_doc, id, item, set_company};
use common::members::{B_TOKEN, as_beta};
use common::{TestDb, call, router};
use invoiceapp::migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};
use serde_json::json;
use uuid::Uuid;

/// One row of each referenced root table, created through the API.
struct Refs {
    contact: Uuid,
    bank: Uuid,
    category: Uuid,
    doc: Uuid,
}

fn uuid(s: &str) -> Uuid {
    s.parse().expect("uuid")
}

async fn refs(app: &Router) -> Refs {
    set_company(app, true).await;
    let (s, cat) = call(
        app,
        Method::POST,
        "/api/settings/categories",
        Some(json!({ "name": "Služby", "kind": "expense" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{cat}");
    let doc = create_doc(app, json!({ "lines": [item("1", "100", "21")] })).await;
    Refs {
        contact: uuid(&create_contact(app, json!({})).await),
        bank: uuid(&create_bank(app, "CZK").await),
        category: uuid(cat["id"].as_str().expect("category id")),
        doc: uuid(&id(&doc)),
    }
}

async fn set(conn: &DatabaseConnection, column: &str, value: Uuid, doc: Uuid) -> Result<(), DbErr> {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        format!("UPDATE documents SET {column} = $1 WHERE id = $2"),
        [value.into(), doc.into()],
    ))
    .await
    .map(|r| assert_eq!(r.rows_affected(), 1, "{column}"))
}

#[tokio::test]
async fn cross_space_references_are_refused_by_the_database() {
    let db = TestDb::new().await;
    seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    let a = router(db.conn.clone());
    let b = as_beta(&a);
    let alpha = refs(&a).await;
    let beta = refs(&b).await;
    let target = create_doc(&b, json!({ "lines": [item("1", "1", "21")] })).await;
    let target = uuid(&id(&target));

    for (column, foreign, own) in [
        ("contact_id", alpha.contact, beta.contact),
        ("bank_account_id", alpha.bank, beta.bank),
        ("category_id", alpha.category, beta.category),
        ("related_document_id", alpha.doc, beta.doc),
    ] {
        let err = set(&db.conn, column, foreign, target)
            .await
            .expect_err(column);
        assert!(err.to_string().contains("foreign key"), "{column}: {err}");
        set(&db.conn, column, own, target)
            .await
            .unwrap_or_else(|e| panic!("{column} of the own space: {e}"));
    }

    // `ON DELETE SET NULL (x)` clears only the reference, never `space_id`.
    let (s, _) = call(
        &b,
        Method::DELETE,
        &format!("/api/contacts/{}", beta.contact),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let row = db
        .conn
        .query_one(Statement::from_sql_and_values(
            db.conn.get_database_backend(),
            "SELECT contact_id, space_id FROM documents WHERE id = $1",
            [target.into()],
        ))
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        row.try_get::<Option<Uuid>>("", "contact_id").ok(),
        Some(None)
    );
    assert!(row.try_get::<Uuid>("", "space_id").is_ok());
}

/// Existing cross-space links make the migration fail with their counts,
/// not with a raw FK error; once fixed it applies (and `down` works).
#[tokio::test]
async fn the_migration_refuses_existing_cross_space_links() {
    let db = TestDb::new().await;
    seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    let a = router(db.conn.clone());
    let b = as_beta(&a);
    let alpha = refs(&a).await;
    let beta = refs(&b).await;

    // Back to before the migration (also undoing any later one).
    let all = Migrator::migrations();
    let at = all
        .iter()
        .position(|m| m.name() == "m20261023_000001_tenant_fks")
        .expect("the tenant FK migration");
    let steps = u32::try_from(all.len() - at).expect("steps");
    Migrator::down(&db.conn, Some(steps)).await.expect("down");
    for (column, foreign) in [
        ("contact_id", alpha.contact),
        ("category_id", alpha.category),
    ] {
        set(&db.conn, column, foreign, beta.doc)
            .await
            .unwrap_or_else(|e| panic!("{column} without the FK: {e}"));
    }

    let msg = Migrator::up(&db.conn, None)
        .await
        .expect_err("must refuse")
        .to_string();
    assert!(
        msg.contains("another space") && msg.contains("contact: 1") && msg.contains("category: 1"),
        "{msg}"
    );
    assert!(!msg.contains("bank_account"), "{msg}");

    for (column, own) in [("contact_id", beta.contact), ("category_id", beta.category)] {
        set(&db.conn, column, own, beta.doc)
            .await
            .unwrap_or_else(|e| panic!("fix {column}: {e}"));
    }
    Migrator::up(&db.conn, None)
        .await
        .expect("migrates once fixed");
}
