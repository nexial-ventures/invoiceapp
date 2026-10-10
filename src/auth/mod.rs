//! Users, sessions, personal API tokens and the per-request auth context.
//! Contracts: `docs/api/auth.md`, `docs/api/spaces.md`, `docs/api/mfa.md`,
//! `docs/api/google.md`.

pub mod crypto;
pub mod ctx;
pub mod entity;
pub mod google;
pub mod handlers;
pub mod host;
pub mod mail;
pub mod mfa;
pub mod ratelimit;
pub mod resolve;
pub mod session;
pub mod tokens;
pub mod users;

use axum::Router;
use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};

use crate::app::AppState;
use crate::error::AppError;
use handlers::{login, password, register};
use host::HostCtx;

pub use ctx::{Access, Authed, Manage, Own, Read, Scope, Via, Write};

/// The unauthenticated auth routes (paths relative to `/api`). Their
/// `Origin`, when present, must be the request's own (a cross-site form post
/// is refused).
pub fn public_router() -> Router<AppState> {
    let base_only = Router::new()
        .route("/auth/register", post(register::register))
        .route("/auth/verify", post(register::verify))
        .route("/auth/verify/resend", post(register::resend))
        .layer(middleware::from_fn(host::base_only));
    Router::new()
        .route("/auth/login", post(login::login))
        .route("/auth/login/mfa", post(mfa::login::login_mfa))
        .route("/auth/password-reset", post(password::request_reset))
        .route(
            "/auth/password-reset/confirm",
            post(password::confirm_reset),
        )
        .merge(base_only)
        .layer(middleware::from_fn(origin_if_present))
        // Browser navigations (Google, back from Google): no `Origin` rule.
        .merge(google::public_router())
        .route("/context", get(host::context))
}

/// Account routes of any known host (session or token).
pub fn account_router() -> Router<AppState> {
    Router::new()
        .route("/auth/logout", post(login::logout))
        .route("/auth/me", get(login::me))
        .route("/auth/sessions/revoke-others", post(login::revoke_others))
        .route("/account/password", post(password::change))
        .merge(mfa::account_router())
        .merge(google::account_router())
}

pub async fn origin_if_present(req: Request, next: Next) -> Result<Response, AppError> {
    let host = req
        .extensions()
        .get::<HostCtx>()
        .ok_or(AppError::NotFound)?;
    ctx::check_origin(req.headers(), host, false)?;
    Ok(next.run(req).await)
}
