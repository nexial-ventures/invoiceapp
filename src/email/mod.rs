//! E-mail: SMTP sending of issued documents ([`sender`]), MiniJinja
//! templates with embedded defaults and storage overrides ([`templates`]),
//! the template context ([`context`]) and the per-document log ([`log`]).

pub mod config;
pub mod context;
pub mod entity;
pub mod handlers;
pub mod input;
pub mod limits;
pub mod log;
pub mod sender;
pub mod templates;

use axum::Router;
use axum::routing::{get, post, put};

use crate::app::AppState;
pub use config::SmtpConfig;
use handlers::{document, settings};
pub use sender::Mailer;

/// Routes relative to `/api/settings/email`.
pub fn settings_router() -> Router<AppState> {
    Router::new()
        .route("/", get(settings::status))
        .route("/test", post(settings::test))
        .route("/templates", get(settings::list_templates))
        .route(
            "/templates/{locale}",
            put(settings::put_template).delete(settings::delete_template),
        )
        .route("/templates/{locale}/preview", post(settings::preview))
}

/// Routes relative to `/api/documents/{id}`.
pub fn document_router() -> Router<AppState> {
    Router::new()
        .route("/{id}/email", get(document::prefill).post(document::send))
        .route("/{id}/emails", get(document::history))
}
