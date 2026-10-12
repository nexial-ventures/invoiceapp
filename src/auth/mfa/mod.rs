//! TOTP two-factor authentication (4c). Contract: `docs/api/mfa.md`.
//!
//! `totp` (RFC 6238 + base32, pure), `seal` (AES-256-GCM secrets and the
//! recovery-code HMAC under `INVOICE__SECRET_KEY`), `repo` (TOTP columns of
//! `users`, `recovery_codes`, `mfa_logins`), `verify` (code checks in the
//! code bucket, the step-up rule), `handlers` (`/api/account/mfa*`),
//! `login` (`POST /api/auth/login/mfa`, the `invoice_mfa` cookie).

pub mod handlers;
pub mod login;
pub mod repo;
pub mod seal;
pub mod totp;
pub mod verify;

use axum::Router;
use axum::routing::{get, post};

use crate::app::AppState;

pub use login::{PENDING_COOKIE, clear_pending, pending_cookie};
pub use verify::step_up;

/// Account routes relative to `/api` (any known host, authenticated).
pub fn account_router() -> Router<AppState> {
    Router::new()
        .route("/account/mfa", get(handlers::status))
        .route("/account/mfa/setup", post(handlers::setup))
        .route("/account/mfa/enable", post(handlers::enable))
        .route("/account/mfa/disable", post(handlers::disable))
        .route("/account/mfa/recovery-codes", post(handlers::regenerate))
}
