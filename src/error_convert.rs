//! Database errors and request rejections → [`AppError`].

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use sea_orm::{DbErr, RuntimeErr, TransactionError, sqlx};

use super::AppError;

/// The name of the unique constraint or index `err` violated, if it is a
/// unique violation (empty when the driver reports no name).
pub fn unique_violation(err: &DbErr) -> Option<String> {
    let (DbErr::Exec(RuntimeErr::SqlxError(sqlx::Error::Database(e)))
    | DbErr::Query(RuntimeErr::SqlxError(sqlx::Error::Database(e)))) = err
    else {
        return None;
    };
    e.is_unique_violation()
        .then(|| e.constraint().unwrap_or_default().to_string())
}

/// Maps a violation of the unique document-number indexes to `on_number`,
/// any other error as usual.
pub fn number_violation(err: DbErr, on_number: impl FnOnce() -> AppError) -> AppError {
    match unique_violation(&err).as_deref() {
        Some("documents_issued_number_key" | "documents_received_number_key") => on_number(),
        _ => err.into(),
    }
}

impl From<DbErr> for AppError {
    /// Unique violations nobody mapped to a field (typically a race on a
    /// partial unique index) are a client-visible 409, not a 500.
    fn from(err: DbErr) -> Self {
        match unique_violation(&err) {
            Some(msg) => Self::Conflict(msg),
            None => Self::Database(err),
        }
    }
}

/// For `db.transaction(|txn| …)`, which awaits the rollback when the closure
/// fails — a dropped `DatabaseTransaction` only queues it.
impl From<TransactionError<AppError>> for AppError {
    fn from(err: TransactionError<AppError>) -> Self {
        match err {
            TransactionError::Connection(e) => e.into(),
            TransactionError::Transaction(e) => e,
        }
    }
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        Self::BadRequest(rejection.body_text())
    }
}

impl From<QueryRejection> for AppError {
    fn from(rejection: QueryRejection) -> Self {
        Self::BadRequest(rejection.body_text())
    }
}

/// A path segment that does not parse (e.g. a non-UUID id) cannot name an
/// existing resource.
impl From<PathRejection> for AppError {
    fn from(_: PathRejection) -> Self {
        Self::NotFound
    }
}

/// `SELECT … FOR UPDATE NOWAIT` found the row locked (SQLSTATE `55P03`).
pub fn lock_not_available(err: &DbErr) -> bool {
    let (DbErr::Exec(RuntimeErr::SqlxError(sqlx::Error::Database(e)))
    | DbErr::Query(RuntimeErr::SqlxError(sqlx::Error::Database(e)))) = err
    else {
        return false;
    };
    e.code().as_deref() == Some("55P03")
}
