//! The `doc_type` CHECK migration refuses rows outside the enum, then the
//! CHECKs reject them.

mod common;

use invoice::migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};

/// Migrations before `m20261025_000001_doc_type_checks`.
const BEFORE: u32 = 19;

const SPACE: &str = "00000000-0000-0000-0000-000000000001";

async fn exec(conn: &DatabaseConnection, sql: &str) {
    conn.execute_unprepared(sql).await.expect(sql);
}

fn document(doc_type: &str) -> String {
    format!(
        "INSERT INTO documents (id, space_id, direction, doc_type, issue_date, due_date, currency, \
         locale, vat_mode, payment_method) VALUES (gen_random_uuid(), '{SPACE}', 'received', \
         '{doc_type}', '2026-01-01', '2026-01-15', 'CZK', 'cs', 'standard', 'bank_transfer')"
    )
}

fn series(doc_type: &str) -> String {
    format!(
        "INSERT INTO number_series (id, space_id, doc_type, pattern) \
         VALUES (gen_random_uuid(), '{SPACE}', '{doc_type}', 'X{{NNNN}}')"
    )
}

#[tokio::test]
async fn doc_type_checks_migration() {
    let url = common::test_database_url();
    let schema = common::unique_schema();
    let admin = Database::connect(&url).await.expect("connect");
    exec(&admin, &format!("CREATE SCHEMA \"{schema}\"")).await;
    let mut opts = ConnectOptions::new(url.clone());
    opts.set_schema_search_path(schema.clone())
        .max_connections(2)
        .sqlx_logging(false);
    let conn = Database::connect(opts).await.expect("connect schema");
    Migrator::up(&conn, Some(BEFORE))
        .await
        .expect("earlier migrations");
    exec(
        &conn,
        &format!("INSERT INTO spaces (id, slug, name) VALUES ('{SPACE}', 's', 'S')"),
    )
    .await;

    // A legacy `received` document type aborts, naming the value.
    exec(&conn, &document("received")).await;
    let err = Migrator::up(&conn, None).await.expect_err("must abort");
    assert!(
        err.to_string().contains("documents.doc_type") && err.to_string().contains(": received."),
        "{err}"
    );
    exec(&conn, "DELETE FROM documents").await;

    // So does an unknown series key.
    exec(&conn, &series("order")).await;
    let err = Migrator::up(&conn, None).await.expect_err("must abort");
    assert!(
        err.to_string().contains("number_series.doc_type") && err.to_string().contains(": order."),
        "{err}"
    );
    exec(&conn, "DELETE FROM number_series").await;

    exec(&conn, &document("invoice")).await;
    exec(&conn, &series("received_simplified")).await;
    Migrator::up(&conn, None)
        .await
        .expect("clean data migrates");
    assert!(
        conn.execute_unprepared(&document("received"))
            .await
            .is_err()
    );
    assert!(conn.execute_unprepared(&series("order")).await.is_err());
    exec(&conn, &document("simplified")).await;
    exec(&conn, &series("received_proforma")).await;

    // And back down: the legacy value is allowed again.
    // Down to just before it, whatever was appended since.
    let after = u32::try_from(Migrator::migrations().len()).expect("count") - BEFORE;
    Migrator::down(&conn, Some(after)).await.expect("down");
    exec(&conn, &series("order")).await;

    conn.close().await.expect("close");
    exec(&admin, &format!("DROP SCHEMA \"{schema}\" CASCADE")).await;
}
