//! Shared state and the top-level Axum router.

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::middleware;
use axum::routing::get;
use sea_orm::DatabaseConnection;
use tower_http::trace::TraceLayer;

use crate::ares::AresClient;
use crate::auth::host::{self, PublicUrl};
use crate::auth::ratelimit::RateLimiter;
use crate::cnb::CnbClient;
use crate::email::Mailer;
use crate::error::AppError;
use crate::pdf::PdfRoot;
use crate::secret::SecretKey;
use crate::{
    ares, auth, catalog, cnb, contact, csvio, document, email, health, isdoc, mcp, members,
    openapi, pdf, security_headers, settings, spa, space,
};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub ares: AresClient,
    pub cnb: CnbClient,
    /// mdcast + storage; `pdf.space(id)` is the per-space view every route uses.
    pub pdf: PdfRoot,
    /// `None` when SMTP is not configured (`INVOICE__SMTP__HOST` unset).
    pub email: Option<Mailer>,
    /// `INVOICE__PUBLIC_URL`: base host, scheme, port.
    pub public: PublicUrl,
    /// `INVOICE__EMAIL_DISABLED`: every outbound e-mail is refused / skipped.
    pub email_disabled: bool,
    /// `INVOICE__REGISTRATION`.
    pub registration: bool,
    /// `INVOICE__MAX_SPACES_PER_USER`.
    pub max_spaces_per_user: u32,
    /// `INVOICE__TRUST_FORWARDED`.
    pub trust_forwarded: bool,
    /// `INVOICE__SECRET_KEY`: TOTP secrets at rest, recovery-code HMAC.
    pub secret_key: SecretKey,
    pub limiter: RateLimiter,
    /// `INVOICE__GOOGLE__*`; `None` = Sign in with Google disabled.
    pub google: Option<auth::google::Google>,
}

/// `/api/health` and `/api/openapi.json` answer on any host without auth.
/// Every other `/api` route is classified by its host first (unknown → 404),
/// then: the public auth routes; account routes (any known host, auth);
/// base-host routes (`/api/spaces`); the public invitation accept routes
/// (space host); space-host routes (everything of phases 1–3, `/api/space`,
/// `/api/tokens`, members and invitations, MCP). Non-API paths fall
/// through to the embedded SPA. Every response carries the security headers
/// (`security_headers`).
pub fn router(state: AppState) -> Router {
    let authenticate = || middleware::from_fn_with_state(state.clone(), auth::ctx::authenticate);

    let space_routes = Router::new()
        .nest("/space", space::space_router())
        .nest("/tokens", auth::tokens::router())
        .merge(members::router())
        .nest("/settings", settings::router())
        .nest("/settings/email", email::settings_router())
        .nest("/contacts", contact::router())
        .route("/ares/{ico}", get(ares::handlers::lookup))
        .nest("/documents", document::router())
        .nest("/catalog", catalog::router())
        .nest("/import/isdoc", isdoc::router())
        .nest("/import/csv", csvio::router())
        .nest("/export", csvio::export_router())
        .nest("/pdf", pdf::router())
        .route("/exchange-rates/{currency}", get(cnb::handlers::get_rate))
        .nest("/mcp", mcp::router(state.clone()))
        .layer(authenticate())
        .layer(middleware::from_fn(host::space_only));

    let base_routes = Router::new()
        .nest("/spaces", space::base_router())
        .layer(authenticate())
        .layer(middleware::from_fn(host::base_only));

    let invite_accept = members::public_router()
        .layer(middleware::from_fn(auth::origin_if_present))
        .layer(middleware::from_fn(host::space_only));

    let hosted = Router::new()
        .merge(auth::public_router())
        .merge(invite_accept)
        .merge(auth::account_router().layer(authenticate()))
        .merge(base_routes)
        .merge(space_routes)
        .fallback(api_not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            host::classify,
        ));

    let api = Router::new()
        .route("/health", get(health::health))
        .route("/openapi.json", get(openapi::openapi_json))
        .merge(hosted);

    Router::new()
        .nest("/api", api)
        .fallback(spa::serve)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::client_ip::warn_untrusted_forwarded,
        ))
        // Path only: query strings can carry secrets (the SPA's `/verify?token=…`,
        // `/reset?token=…` pages), which must never reach a log.
        .layer(TraceLayer::new_for_http().make_span_with(|req: &Request<Body>| {
            tracing::debug_span!("request", method = %req.method(), path = %req.uri().path())
        }))
        // Outermost: every response gets them, rejections and 404s included.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers::security_headers,
        ))
        .with_state(state)
}

async fn api_not_found() -> AppError {
    AppError::NotFound
}
