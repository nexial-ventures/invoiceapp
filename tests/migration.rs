//! The phase 4a migration refuses to run over existing single-tenant data.

mod common;

use invoiceapp::migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectOptions, ConnectionTrait, Database};

#[tokio::test]
async fn spaces_migration_aborts_when_data_exists() {
    let url = common::test_database_url();
    let schema = common::unique_schema();
    let admin = Database::connect(&url).await.expect("connect");
    admin
        .execute_unprepared(&format!("CREATE SCHEMA \"{schema}\""))
        .await
        .expect("schema");
    let mut opts = ConnectOptions::new(url.clone());
    opts.set_schema_search_path(schema.clone())
        .max_connections(2)
        .sqlx_logging(false);
    let conn = Database::connect(opts).await.expect("connect schema");
    // Everything before 4a, then a contact as an old instance would have.
    Migrator::up(&conn, Some(9))
        .await
        .expect("pre-4a migrations");
    conn.execute_unprepared(
        "INSERT INTO contacts (id, name) VALUES (gen_random_uuid(), 'Odběratel s.r.o.')",
    )
    .await
    .expect("old data");
    let err = Migrator::up(&conn, None).await.expect_err("must abort");
    assert!(err.to_string().contains("no default space"), "{err}");
    // Nothing of 4a was applied.
    let spaces = conn.execute_unprepared("SELECT 1 FROM spaces").await;
    assert!(spaces.is_err(), "the spaces table must not exist");

    // An empty pre-4a database (only the seeds) migrates.
    conn.execute_unprepared("DELETE FROM contacts")
        .await
        .expect("reset");
    Migrator::up(&conn, None)
        .await
        .expect("4a on an empty database");
    let seeds = conn
        .query_one(sea_orm::Statement::from_string(
            conn.get_database_backend(),
            "SELECT (SELECT count(*) FROM company) + (SELECT count(*) FROM vat_rates) \
             + (SELECT count(*) FROM number_series) + (SELECT count(*) FROM accounting_settings) AS n",
        ))
        .await
        .expect("count")
        .and_then(|r| r.try_get::<i64>("", "n").ok());
    assert_eq!(
        seeds,
        Some(0),
        "global seeds removed (they are per space now)"
    );
    conn.close().await.expect("close");
    admin
        .execute_unprepared(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .await
        .expect("drop schema");
}
