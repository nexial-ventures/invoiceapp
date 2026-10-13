//! Document exports ([api/csv.md](../../docs/api/csv.md),
//! [api/accounting.md](../../docs/api/accounting.md)): the snapshot loader
//! and the stored-document reading shared by the CSV and the accounting
//! program files (`load`, `source`: they use neither format), and the
//! routes that pick a format (`handlers`, the only place that uses both
//! [`crate::csvio`] and [`crate::accounting`]).

pub mod handlers;
pub mod load;
pub mod period;
pub mod source;

#[cfg(test)]
pub mod test_doc;

use axum::Router;
use axum::routing::get;

use crate::app::AppState;

/// Routes relative to `/api/export`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/csv", get(handlers::list_csv))
        .route("/accountant", get(handlers::accountant))
}
