//! `/api/account/mfa*`: status, setup → enable, disable, regenerating the
//! recovery codes. Session only (a token → 403 `forbidden`), any known host.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::repo::{self, SpaceRef};
use super::{seal, totp, verify};
use crate::app::AppState;
use crate::auth::crypto;
use crate::auth::ctx::Authed;
use crate::auth::handlers::check_password;
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::ApiJson;
use crate::secret::SecretKey;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MfaStatus {
    pub enabled: bool,
    pub recovery_codes_left: u64,
    /// The user's spaces that require TOTP, by name.
    pub required_by: Vec<SpaceRef>,
}

#[utoipa::path(
    get,
    path = "/api/account/mfa",
    tag = "mfa",
    security(("cookie" = [])),
    responses(
        (status = 200, body = MfaStatus),
        (status = 403, description = "`forbidden` with a token", body = ErrorBody),
    )
)]
pub async fn status(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<Json<MfaStatus>, AppError> {
    authed.require_session()?;
    let id = authed.user.id;
    Ok(Json(MfaStatus {
        enabled: authed.mfa_enabled,
        recovery_codes_left: repo::recovery_left(&state.db, id).await?,
        required_by: repo::required_by(&state.db, id).await?,
    }))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct SetupInput {
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    /// The secret, base32 (the manual key).
    pub secret: String,
    pub otpauth_uri: String,
}

#[utoipa::path(
    post,
    path = "/api/account/mfa/setup",
    tag = "mfa",
    security(("cookie" = [])),
    request_body = SetupInput,
    responses(
        (status = 200, description = "A pending secret (10 min); a new setup replaces it", body = Setup),
        (status = 403, description = "`forbidden` with a token, `email_unverified`", body = ErrorBody),
        (status = 409, description = "`mfa_enabled`", body = ErrorBody),
        (status = 422, description = "`password: invalid | not_set`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket)", body = ErrorBody),
    )
)]
pub async fn setup(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(input): ApiJson<SetupInput>,
) -> Result<Json<Setup>, AppError> {
    authed.require_session()?;
    authed.require_verified()?;
    let user = &authed.user;
    if authed.mfa_enabled {
        return Err(AppError::MfaEnabled);
    }
    if let Err(reason) = check_password(&state, user, input.password).await? {
        return Err(AppError::field("password", reason));
    }
    let secret = crypto::random_bytes::<{ totp::SECRET_LEN }>()?;
    let sealed = seal::seal(&state.secret_key, user.id, &secret)?;
    if !repo::set_pending(&state.db, user.id, sealed).await? {
        return Err(AppError::MfaEnabled);
    }
    let secret = totp::base32(&secret);
    Ok(Json(Setup {
        otpauth_uri: totp::otpauth_uri(&state.public.host, &user.email, &secret),
        secret,
    }))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct EnableInput {
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCodes {
    /// 10 codes `xxxxx-xxxxx`, shown only in this response.
    pub recovery_codes: Vec<String>,
}

/// Fresh codes and what the database keeps of them.
fn fresh_codes(key: &SecretKey) -> Result<(Vec<String>, Vec<String>), AppError> {
    let codes = seal::recovery_codes()?;
    let hashes = codes
        .iter()
        .map(|c| seal::recovery_hash(key, &seal::normalize_code(c)))
        .collect();
    Ok((codes, hashes))
}

#[utoipa::path(
    post,
    path = "/api/account/mfa/enable",
    tag = "mfa",
    security(("cookie" = [])),
    request_body = EnableInput,
    responses(
        (status = 200, description = "TOTP on; the recovery codes", body = RecoveryCodes),
        (status = 403, description = "`forbidden` with a token, `email_unverified`", body = ErrorBody),
        (status = 409, description = "`mfa_enabled`", body = ErrorBody),
        (status = 422, description = "`code: required | expired | invalid`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket)", body = ErrorBody),
    )
)]
pub async fn enable(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(input): ApiJson<EnableInput>,
) -> Result<Json<RecoveryCodes>, AppError> {
    authed.require_session()?;
    authed.require_verified()?;
    let user = &authed.user;
    if authed.mfa_enabled {
        return Err(AppError::MfaEnabled);
    }
    let now = Utc::now();
    let row = repo::totp(&state.db, user.id).await?;
    let pending = row
        .totp_pending
        .filter(|_| row.totp_pending_expires_at.is_some_and(|e| e > now))
        .ok_or_else(|| AppError::field("code", "expired"))?;
    if input.code.trim().is_empty() {
        return Err(AppError::field("code", "required"));
    }
    let step = verify::check_pending(&state, user, &pending, &input.code)
        .await?
        .ok_or_else(|| AppError::field("code", "invalid"))?;
    let (codes, hashes) = fresh_codes(&state.secret_key)?;
    // Replaced or expired between the read and the update: start over.
    if !repo::activate(&state.db, user.id, pending, step, hashes).await? {
        return Err(AppError::field("code", "expired"));
    }
    Ok(Json(RecoveryCodes {
        recovery_codes: codes,
    }))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct ConfirmInput {
    pub password: String,
    /// TOTP or recovery code.
    pub code: Option<String>,
}

/// Password, then (only when it and everything else is right) the code.
async fn confirm(state: &AppState, authed: &Authed, input: ConfirmInput) -> Result<(), AppError> {
    let mut e = FieldErrors::new();
    if let Err(reason) = check_password(state, &authed.user, input.password).await? {
        e.add("password", reason);
    }
    verify::step_up(
        state,
        &authed.user,
        authed.mfa_enabled,
        input.code.as_deref(),
        &mut e,
    )
    .await?;
    e.into_result()
}

#[utoipa::path(
    post,
    path = "/api/account/mfa/disable",
    tag = "mfa",
    security(("cookie" = [])),
    request_body = ConfirmInput,
    responses(
        (status = 204, description = "TOTP off; sessions and tokens in spaces that require it are deleted"),
        (status = 403, description = "`forbidden` with a token", body = ErrorBody),
        (status = 422, description = "`password: invalid | not_set`, `code: required | invalid`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket)", body = ErrorBody),
    )
)]
pub async fn disable(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(input): ApiJson<ConfirmInput>,
) -> Result<StatusCode, AppError> {
    authed.require_session()?;
    confirm(&state, &authed, input).await?;
    // Nothing to turn off: no session, token or pending login is touched.
    if authed.mfa_enabled {
        repo::disable(&state.db, authed.user.id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/account/mfa/recovery-codes",
    tag = "mfa",
    security(("cookie" = [])),
    request_body = ConfirmInput,
    responses(
        (status = 200, description = "New codes; the old ones stop working", body = RecoveryCodes),
        (status = 403, description = "`forbidden` with a token", body = ErrorBody),
        (status = 409, description = "`conflict`: TOTP is off", body = ErrorBody),
        (status = 422, description = "`password: invalid | not_set`, `code: required | invalid`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket)", body = ErrorBody),
    )
)]
pub async fn regenerate(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(input): ApiJson<ConfirmInput>,
) -> Result<Json<RecoveryCodes>, AppError> {
    authed.require_session()?;
    if !authed.mfa_enabled {
        return Err(AppError::Conflict("mfa_disabled".into()));
    }
    confirm(&state, &authed, input).await?;
    let (codes, hashes) = fresh_codes(&state.secret_key)?;
    repo::set_recovery(&state.db, authed.user.id, hashes).await?;
    Ok(Json(RecoveryCodes {
        recovery_codes: codes,
    }))
}
