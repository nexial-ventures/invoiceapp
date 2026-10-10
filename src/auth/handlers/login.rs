//! Login, logout, the current user, revoking other sessions.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::ClientIp;
use crate::app::AppState;
use crate::auth::ctx::Authed;
use crate::auth::entity::user;
use crate::auth::host::HostCtx;
use crate::auth::mfa::repo::Pending;
use crate::auth::ratelimit::{LOGIN_EMAIL, LOGIN_IP};
use crate::auth::session::Method;
use crate::auth::{crypto, mfa, session, users};
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiJson;
use crate::space::{Role, SpaceId, repo as space_repo};

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
}

pub(crate) fn with_cookie(status: StatusCode, cookie: &str) -> Result<Response, AppError> {
    with_cookies(status.into_response(), &[cookie])
}

/// `resp` with one `Set-Cookie` header per entry of `cookies`.
pub(crate) fn with_cookies(mut resp: Response, cookies: &[&str]) -> Result<Response, AppError> {
    for cookie in cookies {
        let value = HeaderValue::from_str(cookie)
            .map_err(|_| anyhow::anyhow!("cookie is not a valid header value"))?;
        resp.headers_mut().append(header::SET_COOKIE, value);
    }
    Ok(resp)
}

/// May `user` (password already verified) sign in on this host? Not
/// disabled; on a space host also verified and a member.
pub(crate) async fn may_sign_in(
    db: &impl ConnectionTrait,
    user: &user::Model,
    space: Option<SpaceId>,
) -> Result<bool, AppError> {
    if user.disabled {
        return Ok(false);
    }
    let Some(sp) = space else {
        return Ok(true);
    };
    Ok(user.email_verified_at.is_some()
        && space_repo::member_role(db, sp, user.id).await?.is_some())
}

/// The answer of a correct password when the user has TOTP: the code step
/// comes next (`POST /api/auth/login/mfa`).
#[derive(Debug, Serialize, ToSchema)]
pub struct MfaChallenge {
    /// Always `required`.
    pub mfa: &'static str,
}

/// Start a session on this host. Every failure is the same 401.
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "auth",
    request_body = LoginInput,
    responses(
        (status = 200, description = "The user has TOTP: `Set-Cookie: invoice_mfa=…`, continue with `/api/auth/login/mfa`", body = MfaChallenge),
        (status = 204, description = "Logged in; `Set-Cookie: invoice_session=…`"),
        (status = 401, description = "`invalid_credentials`", body = ErrorBody),
        (status = 403, description = "`mfa_required`: the space requires TOTP and the user has none", body = ErrorBody),
        (status = 429, description = "`rate_limited` (+ `Retry-After`)", body = ErrorBody),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Extension(host): Extension<HostCtx>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    ApiJson(input): ApiJson<LoginInput>,
) -> Result<Response, AppError> {
    let email = users::normalize_email(&input.email);
    // Reserved before the slow part, so a parallel burst cannot pass the
    // limit; only failures keep their reservation.
    let buckets = [(LOGIN_EMAIL, email.as_str()), (LOGIN_IP, ip.as_str())];
    state.limiter.reserve(&buckets)?;
    let user = if email.is_empty() {
        None
    } else {
        users::find_by_email(&state.db, &email).await?
    };
    // An account without a password (Google only) runs the dummy
    // verification: refused like a wrong password.
    let ok = crypto::verify_password(
        input.password,
        user.as_ref().and_then(|u| u.password_hash.clone()),
    )
    .await?;
    let space = host.space_id();
    let user = match user {
        Some(u) if ok && may_sign_in(&state.db, &u, space).await? => u,
        _ => return Err(AppError::InvalidCredentials),
    };
    state.limiter.refund(&buckets);
    let ua = session::user_agent(&headers);
    let pending = Pending::login(Method::Password);
    if let Some(value) = second_factor(&state, &host, user.id, ua.clone(), pending).await? {
        let resp = Json(MfaChallenge { mfa: "required" }).into_response();
        return with_cookies(resp, &[&mfa::pending_cookie(&value, state.public.https)]);
    }
    let value = session::create(&state.db, user.id, space, ua, Method::Password).await?;
    with_cookie(
        StatusCode::NO_CONTENT,
        &session::set_cookie(&value, state.public.https),
    )
}

/// The TOTP rule after a correct first factor (password or Google), the
/// same for both: a user with TOTP gets a pending login (`Some(cookie
/// value)`, the code step comes next) on every host; a user without TOTP on
/// a space that requires it → 403 `mfa_required`; otherwise `None` — the
/// caller starts the session.
pub(crate) async fn second_factor(
    state: &AppState,
    host: &HostCtx,
    user_id: Uuid,
    ua: Option<String>,
    pending: Pending,
) -> Result<Option<String>, AppError> {
    if mfa::repo::mfa_enabled(&state.db, user_id).await? {
        let space = host.space_id();
        return mfa::repo::start_login(&state.db, user_id, space, ua, pending)
            .await
            .map(Some);
    }
    if host.requires_mfa() {
        return Err(AppError::MfaRequired {
            account_created: false,
        });
    }
    Ok(None)
}

/// End this session (a token request has none: 204 all the same).
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "auth",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 204, description = "Session deleted, cookie cleared"))
)]
pub async fn logout(State(state): State<AppState>, authed: Authed) -> Result<Response, AppError> {
    if let Ok(id) = authed.require_session() {
        session::delete(&state.db, id).await?;
    }
    with_cookie(
        StatusCode::NO_CONTENT,
        &session::clear_cookie(state.public.https),
    )
}

/// Delete every other session of the user, on all hosts (session only).
#[utoipa::path(
    post,
    path = "/api/auth/sessions/revoke-others",
    tag = "auth",
    security(("cookie" = [])),
    responses((status = 204), (status = 403, description = "`forbidden` with a token", body = ErrorBody))
)]
pub async fn revoke_others(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<StatusCode, AppError> {
    let current = authed.require_session()?;
    session::delete_others(&state.db, authed.user.id, Some(current)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub email_verified: bool,
    pub mfa_enabled: bool,
    /// `false`: an account created through Google that has not set a
    /// password yet (password-requiring actions answer `password: not_set`).
    pub has_password: bool,
    /// A Google account is linked (`docs/api/google.md`).
    pub google_linked: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeSpace {
    pub slug: String,
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Me {
    pub user: MeUser,
    pub space: Option<MeSpace>,
}

#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "auth",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, body = Me), (status = 401, body = ErrorBody))
)]
pub async fn me(authed: Authed) -> Json<Me> {
    let mfa_enabled = authed.mfa_enabled;
    Json(Me {
        user: MeUser {
            id: authed.user.id,
            email: authed.user.email,
            display_name: authed.user.display_name,
            email_verified: authed.user.email_verified_at.is_some(),
            mfa_enabled,
            has_password: authed.user.password_hash.is_some(),
            google_linked: authed.user.google_sub.is_some(),
        },
        space: authed.space.map(|m| MeSpace {
            slug: m.slug,
            name: m.name,
            role: m.role,
        }),
    })
}
