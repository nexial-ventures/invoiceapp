//! Documents: invoices, simplified tax documents, proformas, DDPPs, credit /
//! debit notes, DDPP corrections — lines, VAT recap,
//! issue, cancel, payments, advance settlement.

pub mod advance;
pub mod compute;
pub mod correction;
pub mod credit;
pub mod custom_fields;
pub mod ddpp;
pub mod defaults;
pub mod entity;
pub mod handlers;
pub mod line;
pub mod received;
pub mod repo;
pub mod rounding;
pub mod state;
pub mod subtotals;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};

use crate::app::AppState;
use handlers::{actions, documents, original, payments};

/// Routes relative to `/api/documents`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(documents::list).post(documents::create))
        .route("/compute", post(documents::compute))
        .route("/isdoc", get(crate::isdoc::export::bulk))
        .route(
            "/{id}",
            get(documents::get)
                .put(documents::update)
                .delete(documents::delete),
        )
        .route("/{id}/issue", post(actions::issue))
        .route("/{id}/pdf", get(crate::pdf::handlers::document_pdf))
        .route("/{id}/isdoc", get(crate::isdoc::export::document_isdoc))
        .route("/{id}/cancel", post(actions::cancel))
        .route("/{id}/mark-sent", post(actions::mark_sent))
        .route("/{id}/internal-note", put(actions::internal_note))
        .route("/{id}/metadata", put(actions::metadata))
        .route(
            "/{id}/original",
            put(original::put)
                .delete(original::delete)
                .layer(DefaultBodyLimit::max(original::BODY_LIMIT)),
        )
        .route("/{id}/settle", post(actions::settle))
        .route("/{id}/credit-note", post(actions::credit_note))
        .route("/{id}/debit-note", post(actions::debit_note))
        .route("/{id}/payments", get(payments::list).post(payments::create))
        .route("/{id}/payments/{payment_id}", delete(payments::delete))
        .merge(crate::email::document_router())
}
