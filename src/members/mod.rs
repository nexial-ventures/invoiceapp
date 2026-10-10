//! Members of a space and invitations into it (4b). Contract:
//! `docs/api/members.md`.

pub mod accept;
pub mod entity;
pub mod handlers;
pub mod invite_handlers;
pub mod invites;
pub mod join;
pub mod repo;
pub mod rules;

use axum::Router;
use axum::routing::{delete, get, post, put};

use crate::app::AppState;

/// Admin+ routes relative to `/api` (space host, authenticated).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/members", get(handlers::list))
        .route(
            "/members/{userId}",
            put(handlers::update).delete(handlers::remove),
        )
        .route(
            "/invites",
            get(invite_handlers::list).post(invite_handlers::create),
        )
        .route("/invites/{id}", delete(invite_handlers::revoke))
        .route("/invites/{id}/resend", post(invite_handlers::resend))
}

/// The unauthenticated accept routes relative to `/api` (space host);
/// `Origin`, when sent, must be the request's own.
pub fn public_router() -> Router<AppState> {
    Router::new().route("/invites/accept", get(accept::show).post(accept::accept))
}
