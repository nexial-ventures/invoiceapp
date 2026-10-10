//! Sign in with Google (4d): OpenID Connect, authorization code + PKCE.
//! Contract: `docs/api/google.md`.
//!
//! `config` (`INVOICE__GOOGLE__*`), `provider` (discovery, JWKS cache, code
//! exchange), `idtoken` (ID token checks, pure), `flow` (flow + handoff rows,
//! PKCE, the `return` rule), `account` (resolving / linking users, unlink),
//! `handlers` (`start`, `callback`), `finish` (the origin host's half).

pub mod account;
pub mod config;
pub mod entity;
pub mod finish;
pub mod flow;
pub mod handlers;
pub mod idtoken;
pub mod jwks;
pub mod provider;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use uuid::Uuid;

pub use config::GoogleConfig;
pub use flow::Purpose;
pub use provider::Google;

use crate::app::AppState;
use crate::auth::host::{self, HostCtx};
use crate::error::AppError;
use crate::space::{SpaceId, repo as space_repo};

/// Host-only cookie set by `start` on the origin host and checked by
/// `finish`: only the browser that started a flow can complete it.
pub const BROWSER_COOKIE: &str = "invoice_google";

/// Why a sign-in ended: the `google_error` code of the redirect. Details are
/// logged, never put in the URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    Cancelled,
    Expired,
    Invalid,
    UnverifiedEmail,
    NotAllowed,
    Disabled,
    NotMember,
    MfaRequired,
    AlreadyLinked,
    EmailMismatch,
    Unavailable,
}

impl Fail {
    pub fn code(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Invalid => "invalid",
            Self::UnverifiedEmail => "unverified_email",
            Self::NotAllowed => "not_allowed",
            Self::Disabled => "disabled",
            Self::NotMember => "not_member",
            Self::MfaRequired => "mfa_required",
            Self::AlreadyLinked => "already_linked",
            Self::EmailMismatch => "email_mismatch",
            Self::Unavailable => "unavailable",
        }
    }
}

/// A server-side failure (database…) in the middle of a browser redirect
/// chain: logged, answered as `unavailable`.
impl From<AppError> for Fail {
    fn from(e: AppError) -> Self {
        tracing::error!(error = %e, "Google sign-in failed");
        Self::Unavailable
    }
}

/// `{origin}/login?google_error=…`, `/account?google_error=…` (link) or
/// `/invite?token=…&google_error=…` (invite; without a token → the login
/// page).
pub fn error_location(origin: &str, purpose: Purpose, invite: Option<&str>, fail: Fail) -> String {
    let code = fail.code();
    match (purpose, invite.and_then(flow::url_safe_token)) {
        (Purpose::Link, _) => format!("{origin}/account?google_error={code}"),
        (Purpose::Invite, Some(t)) => format!("{origin}/invite?token={t}&google_error={code}"),
        _ => format!("{origin}/login?google_error={code}"),
    }
}

/// 302 to `location` with `cookies`; never cached, no referrer (the URLs
/// carry single-use tokens).
pub fn redirect(location: &str, cookies: &[&str]) -> Result<Response, AppError> {
    let value = HeaderValue::from_str(location)
        .map_err(|_| anyhow::anyhow!("redirect location is not a valid header value"))?;
    let mut resp = StatusCode::FOUND.into_response();
    let h = resp.headers_mut();
    h.insert(header::LOCATION, value);
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    crate::auth::handlers::login::with_cookies(resp, cookies)
}

/// The only redirect URI: the callback on the base host.
pub fn callback_uri(state: &AppState) -> String {
    format!("{}/api/auth/google/callback", state.public.base_url())
}

/// The URL of the request's own host.
pub fn host_url(state: &AppState, host: &HostCtx) -> String {
    match &host.space {
        Some(s) => state.public.space_url(&s.slug),
        None => state.public.base_url(),
    }
}

/// The URL of the host of `space` (`None` = base); `None` when the space
/// is gone.
pub async fn origin_url(state: &AppState, space: Option<Uuid>) -> Result<Option<String>, AppError> {
    let Some(id) = space else {
        return Ok(Some(state.public.base_url()));
    };
    Ok(space_repo::find(&state.db, SpaceId(id))
        .await?
        .map(|s| state.public.space_url(&s.slug)))
}

/// The enabled configuration, else 404 (the routes do not exist).
pub fn enabled(state: &AppState) -> Result<&Google, AppError> {
    state.google.as_ref().ok_or(AppError::NotFound)
}

/// Unauthenticated routes relative to `/api` (browser navigations: no
/// `Origin` check). The callback exists on the base host only.
pub fn public_router() -> Router<AppState> {
    let callback = Router::new()
        .route("/auth/google/callback", get(handlers::callback))
        .layer(middleware::from_fn(host::base_only));
    Router::new()
        .route("/auth/google/start", get(handlers::start))
        .route("/auth/google/finish", get(finish::finish))
        .merge(callback)
}

/// Account routes relative to `/api` (authenticated).
pub fn account_router() -> Router<AppState> {
    Router::new()
        .route("/account/google", delete(account::unlink))
        .route("/account/google/link", post(account::link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_redirects() {
        let o = "http://acme.localhost:3000";
        assert_eq!(
            error_location(o, Purpose::Login, None, Fail::NotMember),
            "http://acme.localhost:3000/login?google_error=not_member"
        );
        assert_eq!(
            error_location(o, Purpose::Link, None, Fail::AlreadyLinked),
            "http://acme.localhost:3000/account?google_error=already_linked"
        );
        assert_eq!(
            error_location(o, Purpose::Invite, Some("tok-1"), Fail::EmailMismatch),
            "http://acme.localhost:3000/invite?token=tok-1&google_error=email_mismatch"
        );
        assert_eq!(
            error_location(o, Purpose::Invite, Some("a&b"), Fail::Invalid),
            "http://acme.localhost:3000/login?google_error=invalid"
        );
        let codes = [
            Fail::Cancelled,
            Fail::Expired,
            Fail::Invalid,
            Fail::UnverifiedEmail,
            Fail::NotAllowed,
            Fail::Disabled,
            Fail::NotMember,
            Fail::MfaRequired,
            Fail::AlreadyLinked,
            Fail::EmailMismatch,
            Fail::Unavailable,
        ]
        .map(Fail::code);
        assert!(
            codes
                .iter()
                .all(|c| c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        );
    }

    #[test]
    fn redirect_headers() {
        let r = redirect("/x", &["a=1; Path=/"]).expect("redirect");
        assert_eq!(r.status(), StatusCode::FOUND);
        assert_eq!(r.headers()[header::LOCATION], "/x");
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(r.headers()[header::SET_COOKIE], "a=1; Path=/");
        assert!(redirect("/a\nb", &[]).is_err());
    }
}
