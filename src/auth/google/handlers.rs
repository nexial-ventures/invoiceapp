//! `GET /api/auth/google/start` (any known host) and
//! `GET /api/auth/google/callback` (base host). Both are browser
//! navigations: past the 404 (Google off) and the start's rate limit, every
//! failure — internal ones included (logged) — ends in an error redirect.

use axum::Extension;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use super::entity::google_flow;
use super::flow::{self, NewFlow, Purpose};
use super::{Fail, account, callback_uri, enabled, error_location, redirect};
use crate::app::AppState;
use crate::auth::handlers::ClientIp;
use crate::auth::host::HostCtx;
use crate::auth::ratelimit::GOOGLE_IP;
use crate::auth::users::{self, TokenKind};
use crate::auth::{crypto, ctx, session};
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiQuery;
use crate::members::entity::space_invite;
use crate::members::invites;
use crate::space::SpaceId;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct StartQuery {
    /// `login` | `link` (with `grant`) | `invite` (space host).
    pub purpose: String,
    /// Where to land afterwards: a relative path, else `/`.
    #[serde(rename = "return")]
    #[param(rename = "return")]
    pub return_to: Option<String>,
    /// `invite`: the invitation token of the link.
    pub invite: Option<String>,
    /// `link`: the single-use grant of `POST /api/account/google/link`.
    pub grant: Option<String>,
}

/// Start a sign-in with Google: 302 to Google's authorization endpoint.
#[utoipa::path(
    get,
    path = "/api/auth/google/start",
    tag = "auth",
    params(StartQuery),
    responses(
        (status = 302, description = "To Google (`Set-Cookie: invoice_google_…=…`, this flow's browser binding), or an error redirect `{host}/login?google_error=invalid | unavailable` (`/account?…` for link, `/invite?token=…&…` for invite)"),
        (status = 404, description = "Sign in with Google is not configured", body = ErrorBody),
        (status = 429, description = "`rate_limited` (20 / 15 min per IP)", body = ErrorBody),
    )
)]
pub async fn start(
    State(state): State<AppState>,
    Extension(host): Extension<HostCtx>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<StartQuery>,
) -> Result<Response, AppError> {
    let google = enabled(&state)?.clone();
    state.limiter.reserve(&[(GOOGLE_IP, ip.as_str())])?;
    let origin = super::host_url(&state, &host);
    let purpose = Purpose::parse(q.purpose.trim());
    let token = q
        .invite
        .as_deref()
        .map(str::trim)
        .and_then(flow::url_safe_token);
    match begin(&state, &google, &host, &headers, purpose, &q, token).await {
        Ok((url, cookie)) => redirect(&url, &[&cookie]),
        Err(f) => {
            let shown = purpose.unwrap_or(Purpose::Login);
            redirect(&error_location(&origin, shown, token, f), &[])
        }
    }
}

/// The flow row and the authorization URL; returns it with the binding
/// cookie.
async fn begin(
    state: &AppState,
    google: &super::Google,
    host: &HostCtx,
    headers: &HeaderMap,
    purpose: Option<Purpose>,
    q: &StartQuery,
    token: Option<&str>,
) -> Result<(String, String), Fail> {
    let purpose = purpose.ok_or(Fail::Invalid)?;
    let user_id = match purpose {
        Purpose::Link => Some(link_user(state, host, headers, q.grant.as_deref()).await?),
        _ => None,
    };
    let invite = match (purpose, host.space_id(), token) {
        (Purpose::Invite, Some(space), Some(t)) => {
            invites::by_token(&state.db, space, t)
                .await?
                .ok_or(Fail::Invalid)?;
            Some(t.to_string())
        }
        (Purpose::Invite, ..) => return Err(Fail::Invalid),
        _ => None,
    };
    let browser = crypto::random_token()?;
    let new = NewFlow {
        space: host.space_id(),
        purpose,
        return_path: flow::sanitize_return(q.return_to.as_deref()),
        invite,
        user_id,
        browser: browser.clone(),
    };
    let started = flow::create(&state.db, &state.secret_key, new).await?;
    let challenge = flow::challenge(&started.verifier);
    let url = google
        .authorize_url(
            &callback_uri(state),
            &started.state,
            &challenge,
            &started.nonce,
        )
        .await?;
    let https = state.public.https;
    let cookie = session::cookie(&started.cookie_name, &browser, flow::FLOW_TTL, https);
    Ok((url, cookie))
}

/// `link`: the session on this host and its single-use grant (password
/// and step-up already confirmed by `POST /api/account/google/link`).
async fn link_user(
    state: &AppState,
    host: &HostCtx,
    headers: &HeaderMap,
    grant: Option<&str>,
) -> Result<Uuid, Fail> {
    let cookie = session::cookie_value(headers).ok_or(Fail::Invalid)?;
    let user = match ctx::by_session(state, host, &cookie).await {
        Ok(a) => a.user.id,
        Err(AppError::Unauthorized) => return Err(Fail::Invalid),
        Err(e) => return Err(e.into()),
    };
    let grant = grant.map(str::trim).filter(|g| !g.is_empty());
    let grant = grant.ok_or(Fail::Invalid)?;
    let used = users::consume_token(&state.db, grant, TokenKind::GoogleLink, |txn, owner| {
        // Another user's grant is refused and stays unused.
        async move {
            match owner == user {
                true => Ok(txn),
                false => Err(AppError::field("token", "invalid")),
            }
        }
    })
    .await;
    match used {
        Ok(()) => Ok(user),
        Err(AppError::Validation(_)) => Err(Fail::Invalid),
        Err(e) => Err(e.into()),
    }
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct CallbackQuery {
    pub state: String,
    pub code: String,
    /// Set by the provider instead of `code` (`access_denied` = cancelled).
    pub error: Option<String>,
}

/// Google's redirect back (base host): verifies the flow and the ID token
/// and hands the verified claims to the origin host. No account is touched
/// here — that happens in `finish`, after the browser binding.
#[utoipa::path(
    get,
    path = "/api/auth/google/callback",
    tag = "auth",
    params(CallbackQuery),
    responses(
        (status = 302, description = "To `{origin host}/api/auth/google/finish?token=…`, or an error redirect with `google_error` = `cancelled | expired | invalid | unverified_email | already_linked | email_mismatch | unavailable`"),
        (status = 404, description = "Not configured, or not the base host", body = ErrorBody),
    )
)]
pub async fn callback(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<CallbackQuery>,
) -> Result<Response, AppError> {
    let google = enabled(&state)?.clone();
    let base = state.public.base_url();
    let to_base = |f: Fail| redirect(&error_location(&base, Purpose::Login, None, f), &[]);
    let flow = match flow::take_flow(&state.db, q.state.trim()).await {
        Ok(Some(f)) => f,
        Ok(None) => return to_base(Fail::Invalid),
        Err(e) => return to_base(e.into()),
    };
    let purpose = Purpose::parse(&flow.purpose).unwrap_or(Purpose::Login);
    let origin = match super::origin_url(&state, flow.space_id).await {
        Ok(Some(o)) => o,
        Ok(None) => return to_base(Fail::Invalid),
        Err(e) => return to_base(e.into()),
    };
    let invite = flow::open_invite(
        &state.secret_key,
        flow.space_id,
        flow.invite_sealed.as_deref(),
    );
    let fail = |f: Fail| redirect(&error_location(&origin, purpose, invite.as_deref(), f), &[]);
    if flow::expired(flow.expires_at) {
        return fail(Fail::Expired);
    }
    if let Some(error) = q.error.as_deref() {
        tracing::info!(error, "Google sign-in: the provider answered an error");
        return fail(if error == "access_denied" {
            Fail::Cancelled
        } else {
            Fail::Invalid
        });
    }
    match complete(&state, &google, &flow, purpose, q.code.trim()).await {
        Ok(token) => redirect(
            &format!("{origin}/api/auth/google/finish?token={token}"),
            &[],
        ),
        Err(f) => fail(f),
    }
}

/// The ID token, early read-only checks (`finish` repeats them before it
/// writes), the handoff.
async fn complete(
    state: &AppState,
    google: &super::Google,
    flow: &google_flow::Model,
    purpose: Purpose,
    code: &str,
) -> Result<String, Fail> {
    if code.is_empty() {
        return Err(Fail::Invalid);
    }
    let id = google
        .identity(&callback_uri(state), code, &flow.pkce_verifier, &flow.nonce)
        .await?;
    match purpose {
        Purpose::Link => {
            let user = flow.user_id.ok_or(Fail::Invalid)?;
            account::check_link(&state.db, user, &id.sub).await?;
        }
        Purpose::Invite => {
            invitation(state, flow.space_id, flow.invite_hash.as_deref(), &id.email).await?;
        }
        Purpose::Login => {}
    }
    Ok(flow::create_handoff(&state.db, flow, &id).await?)
}

/// The live invitation `hash` of `space`, addressed to `email` (the Google
/// e-mail): gone → `invalid`, another address → `email_mismatch`.
pub(super) async fn invitation(
    state: &AppState,
    space: Option<Uuid>,
    hash: Option<&str>,
    email: &str,
) -> Result<space_invite::Model, Fail> {
    let (Some(space), Some(hash)) = (space, hash) else {
        return Err(Fail::Invalid);
    };
    let invite = invites::by_hash(&state.db, SpaceId::from_row(space), hash)
        .await?
        .ok_or(Fail::Invalid)?;
    if invite.email != email {
        return Err(Fail::EmailMismatch);
    }
    Ok(invite)
}
