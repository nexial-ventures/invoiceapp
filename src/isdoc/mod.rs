//! ISDOC 6.0.2 import (preview → confirm) and export (`.isdoc` / `.isdocx`).

pub mod analyze;
pub mod export;
pub mod export_body;
pub mod export_bulk;
pub mod export_xml;
pub mod import;
pub mod model;
pub mod parse;
pub mod plan;
pub mod upload;
pub mod xml;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;

use crate::app::AppState;

/// Routes relative to `/api/import/isdoc`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/preview", post(import::preview))
        .route("/confirm", post(import::confirm))
        .layer(DefaultBodyLimit::max(crate::import::form::BODY_LIMIT))
}
