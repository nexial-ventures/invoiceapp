//! Password change (session only) and reset by e-mail.

use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

use super::ClientIp;
use super::register::{EmailBody, link, mail_limits};
use crate::app::AppState;
use crate::auth::ctx::Authed;
use crate::auth::mail::{self, Mail};
use crate::auth::ratelimit::RESET_IP;
use crate::auth::users::{self, TokenKind};
use crate::auth::{crypto, mfa, session};
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::ApiJson;

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ChangeInput {
    /// Not needed (ignored) for an account without a password within 10 min
    /// of a Google login.
    pub current_password: String,
    pub new_password: String,
    /// Users with TOTP: a TOTP or recovery code.
    pub code: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/account/password",
    tag = "auth",
    security(("cookie" = [])),
    request_body = ChangeInput,
    responses(
        (status = 204, description = "Changed (or set: an account without a password, within 10 min of a Google login); the user's other sessions, pending logins and API tokens (all spaces) are revoked"),
        (status = 403, description = "`forbidden` with a token, `email_unverified` (use the password reset, which verifies the address)", body = ErrorBody),
        (status = 422, description = "`currentPassword: invalid | required` (`required`: no password yet and no Google login within 10 min), `newPassword: too_short | too_long`, `code: required | invalid` (users with TOTP)", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket)", body = ErrorBody),
    )
)]
pub async fn change(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(input): ApiJson<ChangeInput>,
) -> Result<StatusCode, AppError> {
    let current = authed.require_session()?;
    authed.require_verified()?;
    let mut e = FieldErrors::new();
    e.check("newPassword", super::password(&input.new_password));
    if authed.user.password_hash.is_none() {
        // Setting a first password: the fresh Google login is the proof.
        if !fresh_google(&state, current).await? {
            e.add("currentPassword", "required");
        }
    } else if let Err(reason) =
        super::check_password(&state, &authed.user, input.current_password).await?
    {
        e.add("currentPassword", reason);
    }
    let code = input.code.as_deref();
    mfa::step_up(&state, &authed.user, authed.mfa_enabled, code, &mut e).await?;
    e.into_result()?;
    let hash = crypto::hash_password(input.new_password).await?;
    users::set_password(&state.db, authed.user.id, hash).await?;
    session::delete_others(&state.db, authed.user.id, Some(current)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// How long after a Google login an account without a password may set
/// one without `currentPassword`.
pub const SET_WINDOW: chrono::Duration = chrono::Duration::minutes(10);

/// Was session `id` signed in through Google less than [`SET_WINDOW`] ago?
async fn fresh_google(state: &AppState, id: uuid::Uuid) -> Result<bool, AppError> {
    Ok(session::find(&state.db, id).await?.is_some_and(|s| {
        s.login_method == session::Method::Google.as_str()
            && chrono::Utc::now() - s.created_at.to_utc() < SET_WINDOW
    }))
}

#[utoipa::path(
    post,
    path = "/api/auth/password-reset",
    tag = "auth",
    request_body = EmailBody,
    responses(
        (status = 202, description = "Accepted (always)"),
        (status = 429, description = "`rate_limited`", body = ErrorBody),
    )
)]
pub async fn request_reset(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<EmailBody>,
) -> Result<StatusCode, AppError> {
    let email = users::normalize_email(&body.email);
    mail_limits(&state, &ip, &email)?;
    let Some(user) = users::find_by_email(&state.db, &email)
        .await?
        .filter(|u| !u.disabled)
    else {
        return Ok(StatusCode::ACCEPTED);
    };
    let token = users::issue_token(&state.db, user.id, TokenKind::Reset).await?;
    let locale = mail::locale(body.locale.as_deref());
    mail::send(
        &state,
        &user.email,
        Mail::Reset(link(&state, "reset", &token)),
        locale,
    );
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct ResetInput {
    pub token: String,
    pub password: String,
}

#[utoipa::path(
    post,
    path = "/api/auth/password-reset/confirm",
    tag = "auth",
    request_body = ResetInput,
    responses(
        (status = 204, description = "Password set, e-mail verified, every session, pending login and API token revoked"),
        (status = 422, description = "`token: invalid`, `password: too_short | too_long`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (per IP)", body = ErrorBody),
    )
)]
pub async fn confirm_reset(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(input): ApiJson<ResetInput>,
) -> Result<StatusCode, AppError> {
    state.limiter.reserve(&[(RESET_IP, ip.as_str())])?;
    if let Err(reason) = super::password(&input.password) {
        return Err(AppError::field("password", reason));
    }
    // The token is checked (and consumed, row-locked by its update) first:
    // argon2 runs only for a live token, never for a guess.
    users::consume_token(
        &state.db,
        input.token.trim(),
        TokenKind::Reset,
        |txn, user_id| async move {
            let hash = crypto::hash_password(input.password).await?;
            users::set_password(&txn, user_id, hash).await?;
            users::mark_verified(&txn, user_id).await?;
            session::delete_others(&txn, user_id, None).await?;
            Ok(txn)
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
