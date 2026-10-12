//! `/api/spaces` (base host: my spaces, create) and `/api/space` (space
//! host: the current space, update (name, TOTP policy), delete).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::entity::space;
use super::limits::{self, StorageInfo};
use super::{Role, repo, slug};
use crate::app::AppState;
use crate::auth::handlers::check_password;
use crate::auth::{Authed, Manage, Own, Read, mfa};
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::ApiJson;
use crate::pdf::editor::ensure_usage;
use crate::validation as v;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpaceInfo {
    pub slug: String,
    pub name: String,
    pub role: Role,
    /// `{scheme}://{slug}.{base host}{:port}`.
    pub url: String,
    /// New logins on this host need TOTP (`docs/api/mfa.md`).
    pub require_mfa: bool,
    /// `GET` / `PUT /api/space` only: the hidden limits and the usage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<StorageInfo>,
}

fn info(state: &AppState, s: space::Model, role: Role) -> SpaceInfo {
    SpaceInfo {
        url: state.public.space_url(&s.slug),
        slug: s.slug,
        name: s.name,
        role,
        require_mfa: s.require_mfa,
        limits: None,
    }
}

/// [`info`] of the current space, with its limits.
async fn with_limits(
    state: &AppState,
    s: space::Model,
    role: Role,
) -> Result<Json<SpaceInfo>, AppError> {
    let space = super::SpaceId(s.id);
    // Best effort: an unreachable storage must not fail `GET /api/space`.
    let ensured = match state.pdf.space(space) {
        Ok(pdf) => ensure_usage(&state.db, space, pdf.storage()).await,
        Err(e) => Err(e),
    };
    if let Err(e) = ensured {
        tracing::warn!(space = %space, error = %e, "record the design storage usage");
    }
    let limits = limits::info(&state.db, space).await?;
    Ok(Json(SpaceInfo {
        limits: Some(limits),
        ..info(state, s, role)
    }))
}

#[utoipa::path(
    get,
    path = "/api/spaces",
    tag = "spaces",
    security(("cookie" = [])),
    responses((status = 200, body = Vec<SpaceInfo>), (status = 401, body = ErrorBody))
)]
pub async fn list(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<Json<Vec<SpaceInfo>>, AppError> {
    let rows = repo::list_for_user(&state.db, authed.user.id).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(s, role)| info(&state, s, role))
            .collect(),
    ))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct CreateInput {
    pub slug: String,
    pub name: String,
}

#[utoipa::path(
    post,
    path = "/api/spaces",
    tag = "spaces",
    security(("cookie" = [])),
    request_body = CreateInput,
    responses(
        (status = 201, body = SpaceInfo),
        (status = 403, description = "`space_creation_disabled` (`INVOICE__REGISTRATION` off), `email_unverified`", body = ErrorBody),
        (status = 422, description = "`slug: required | invalid | reserved | taken`, `name: required | too_long`", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(input): ApiJson<CreateInput>,
) -> Result<(StatusCode, Json<SpaceInfo>), AppError> {
    if !state.registration {
        return Err(AppError::SpaceCreationDisabled);
    }
    authed.require_verified()?;
    let mut e = FieldErrors::new();
    let slug = e.check("slug", slug::validate(&input.slug));
    let name = e.check("name", v::required_text(&input.name, 200));
    e.into_result()?;
    let row = repo::create(
        &state.db,
        slug.unwrap_or_default(),
        name.unwrap_or_default(),
        authed.user.id,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(info(&state, row, Role::Owner))))
}

async fn current(state: &AppState, access_space: super::SpaceId) -> Result<space::Model, AppError> {
    repo::find(&state.db, access_space)
        .await?
        .ok_or(AppError::NotFound)
}

#[utoipa::path(
    get,
    path = "/api/space",
    tag = "spaces",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, body = SpaceInfo))
)]
pub async fn get(State(state): State<AppState>, access: Read) -> Result<Json<SpaceInfo>, AppError> {
    let row = current(&state, access.space()).await?;
    with_limits(&state, row, access.scope.role).await
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateInput {
    /// Omitted → unchanged.
    pub name: Option<String>,
    /// Owner only, session only, needs the owner's own TOTP and a step-up
    /// `code` (on and off). Omitted → unchanged.
    pub require_mfa: Option<bool>,
    /// TOTP or recovery code; required with `requireMfa`.
    pub code: Option<String>,
}

#[utoipa::path(
    put,
    path = "/api/space",
    tag = "spaces",
    security(("cookie" = []), ("bearer" = [])),
    request_body = UpdateInput,
    responses(
        (status = 200, body = SpaceInfo),
        (status = 403, description = "`forbidden` below admin, or `requireMfa` sent by a non-owner or with a token", body = ErrorBody),
        (status = 422, description = "`name: required | too_long`, `requireMfa: mfa_not_enabled`, `code: required | invalid`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket, code checks)", body = ErrorBody),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    access: Manage,
    authed: Authed,
    ApiJson(input): ApiJson<UpdateInput>,
) -> Result<Json<SpaceInfo>, AppError> {
    if input.require_mfa.is_some() {
        if access.scope.role < Role::Owner {
            return Err(AppError::Forbidden);
        }
        authed.require_session()?;
    }
    let mut e = FieldErrors::new();
    let name = match input.name.as_deref() {
        Some(raw) => e.check("name", v::required_text(raw, 200)),
        None => None,
    };
    if input.require_mfa.is_some() {
        if authed.mfa_enabled {
            let code = input.code.as_deref();
            mfa::step_up(&state, &authed.user, true, code, &mut e).await?;
        } else {
            e.add("requireMfa", "mfa_not_enabled");
        }
    }
    e.into_result()?;
    let row = repo::update(&state.db, access.space(), name, input.require_mfa).await?;
    with_limits(&state, row, access.scope.role).await
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct DeleteInput {
    pub slug: String,
    pub password: String,
    /// Users with TOTP: a TOTP or recovery code.
    pub code: Option<String>,
}

#[utoipa::path(
    delete,
    path = "/api/space",
    tag = "spaces",
    security(("cookie" = [])),
    request_body = DeleteInput,
    responses(
        (status = 204, description = "The space, all its data and files are deleted"),
        (status = 403, description = "`forbidden`: not the owner, or a token", body = ErrorBody),
        (status = 422, description = "`slug: mismatch`, `password: invalid | not_set`, `code: required | invalid` (users with TOTP)", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket)", body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    access: Own,
    authed: Authed,
    ApiJson(input): ApiJson<DeleteInput>,
) -> Result<StatusCode, AppError> {
    authed.require_session()?;
    let space = current(&state, access.space()).await?;
    let mut e = FieldErrors::new();
    if input.slug.trim() != space.slug {
        e.add("slug", "mismatch");
    }
    if let Err(reason) = check_password(&state, &authed.user, input.password).await? {
        e.add("password", reason);
    }
    let code = input.code.as_deref();
    mfa::step_up(&state, &authed.user, authed.mfa_enabled, code, &mut e).await?;
    e.into_result()?;
    let space = access.space();
    repo::delete(&state.db, space).await?;
    // Detached from the request: a client that disconnects after the commit
    // must not leave the files behind.
    let storage = state.pdf.root_storage().clone();
    tokio::spawn(async move {
        let prefix = space.storage_prefix();
        match storage.delete_prefix(&prefix).await {
            Ok(n) => tracing::info!(space = %space, files = n, "space deleted"),
            Err(e) => tracing::error!(
                space = %space,
                error = %e,
                "space deleted, but its files were not removed (orphaned under {prefix}/)"
            ),
        }
    });
    Ok(StatusCode::NO_CONTENT)
}
