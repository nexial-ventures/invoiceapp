//! The server pool's connections carry the server-side timeouts, unless
//! they go through PgBouncer.

mod common;

use invoice::db::{SERVER_TIMEOUT, pool_options};
use sea_orm::{ConnectionTrait, Database, Statement, TransactionTrait};

async fn show(db: &impl ConnectionTrait, setting: &str) -> String {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            format!("SHOW {setting}"),
        ))
        .await
        .expect("show")
        .expect("one row");
    row.try_get_by_index(0).expect("text")
}

#[tokio::test]
async fn pooled_connections_time_out_statements_and_idle_transactions() {
    let db = Database::connect(pool_options(&common::test_database_url(), 2, false))
        .await
        .expect("connect");
    assert_eq!(show(&db, "statement_timeout").await, SERVER_TIMEOUT);
    let txn = db.begin().await.expect("begin");
    assert_eq!(
        show(&txn, "idle_in_transaction_session_timeout").await,
        SERVER_TIMEOUT
    );
    txn.rollback().await.expect("rollback");
}

#[tokio::test]
async fn pgbouncer_pool_sends_no_startup_options() {
    let db = Database::connect(pool_options(&common::test_database_url(), 2, true))
        .await
        .expect("connect");
    assert_ne!(show(&db, "statement_timeout").await, SERVER_TIMEOUT);
    assert_ne!(
        show(&db, "idle_in_transaction_session_timeout").await,
        SERVER_TIMEOUT
    );
}
