//! The server's connection pool: bounded size and waits, and server-side
//! timeouts so a stuck statement or an abandoned transaction cannot pin a
//! connection (and its row locks) indefinitely.

use std::time::Duration;

use sea_orm::{ConnectOptions, ConnectionTrait, DatabaseTransaction, DbErr, Statement};

/// `INVOICEAPP__DB_MAX_CONNECTIONS` default.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 20;
/// Longest wait for a free pooled connection.
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
/// `statement_timeout` and `idle_in_transaction_session_timeout` of every
/// pooled connection (Postgres `options`, set at connection start).
pub const SERVER_TIMEOUT: &str = "30s";

/// `pgbouncer`: a transaction-pooling PgBouncer refuses startup `options` it
/// does not track and silently drops the ones it does (Postgres does not
/// report these two settings), so the pool goes without the server timeouts.
pub fn pool_options(url: &str, max_connections: u32, pgbouncer: bool) -> ConnectOptions {
    let mut opts = ConnectOptions::new(url.to_string());
    opts.max_connections(max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT);
    if !pgbouncer {
        opts.map_sqlx_postgres_opts(|o| {
            o.options([
                ("statement_timeout", SERVER_TIMEOUT),
                ("idle_in_transaction_session_timeout", SERVER_TIMEOUT),
            ])
        });
    }
    opts
}

/// Lift the idle-in-transaction timeout for this transaction only: one
/// that deliberately waits on the storage between its statements (the
/// design editor's lock).
pub async fn allow_idle(txn: &DatabaseTransaction) -> Result<(), DbErr> {
    txn.execute_unprepared("SET LOCAL idle_in_transaction_session_timeout = 0")
        .await
        .map(drop)
}

/// Serialize on `key` until the transaction ends (a transaction-scoped
/// Postgres advisory lock).
pub async fn advisory_xact_lock(txn: &impl ConnectionTrait, key: String) -> Result<(), DbErr> {
    txn.execute(Statement::from_sql_and_values(
        txn.get_database_backend(),
        "SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))",
        [key.into()],
    ))
    .await
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_is_bounded() {
        let opts = pool_options("postgres://x", 7, false);
        assert_eq!(opts.get_max_connections(), Some(7));
        assert_eq!(opts.get_acquire_timeout(), Some(ACQUIRE_TIMEOUT));
    }
}
