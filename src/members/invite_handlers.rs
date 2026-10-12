//! `/api/invites` (admin+): list, invite (or replace), resend, revoke.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::invites::{self, InviteRow};
use super::rules;
use crate::app::AppState;
use crate::auth::mail::{self, Mail};
use crate::auth::ratelimit::{INVITE_SPACE, INVITE_USER};
use crate::auth::{Authed, Manage, Scope, users};
use crate::email::limits::{self as email_limits, Kind};
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::{ApiJson, ApiPath, optional_json};
use crate::space::{Role, repo as space_repo};

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Inviter {
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub id: Uuid,
    pub email: String,
    pub role: Role,
    pub invited_by: Inviter,
    pub created_at: DateTime<FixedOffset>,
    pub expires_at: DateTime<FixedOffset>,
}

impl From<InviteRow> for Invite {
    fn from(r: InviteRow) -> Self {
        Self {
            id: r.id,
            email: r.email,
            role: r.role,
            invited_by: Inviter {
                email: r.inviter_email,
                display_name: r.inviter_name,
            },
            created_at: r.created_at,
            expires_at: r.expires_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SentInvite {
    #[serde(flatten)]
    pub invite: Invite,
    /// `{space url}/invite?token=…`, only when SMTP is **not** configured
    /// (shown only in this response). With SMTP the link travels only by
    /// e-mail, so accepting proves the invitee owns the address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// `true` = the e-mail is queued (failures are logged), no `url`;
    /// `false` = no SMTP, the `url` is returned instead — or e-mail is
    /// disabled (`INVOICE__EMAIL_DISABLED`, skipped and logged, no `url`:
    /// resend once it is enabled again).
    pub email_sent: bool,
    /// The e-mail was skipped because sending is disabled
    /// (`INVOICE__EMAIL_DISABLED`); resend once it is enabled again.
    pub email_disabled: bool,
}

#[utoipa::path(
    get,
    path = "/api/invites",
    tag = "members",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "Pending, unexpired invitations, newest first", body = Vec<Invite>),
        (status = 403, description = "`forbidden` below admin", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    access: Manage,
) -> Result<Json<Vec<Invite>>, AppError> {
    let rows = invites::list(&state.db, access.space()).await?;
    Ok(Json(rows.into_iter().map(Invite::from).collect()))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct InviteInput {
    pub email: String,
    pub role: Option<String>,
    /// `cs` (default) | `en`: the language of the e-mail.
    pub locale: Option<String>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct ResendInput {
    /// `cs` (default) | `en`.
    pub locale: Option<String>,
}

/// Every invitation (create, resend) counts per inviting user and per
/// space, so the instance SMTP cannot be used as a relay; one that will be
/// mailed also counts in the space's and the user's e-mail limits
/// (`email::limits`; `Some(row)`). A refused request counts in none of them.
async fn invite_limits(state: &AppState, s: Scope) -> Result<Option<i64>, AppError> {
    let (user, space) = (s.user_id.to_string(), s.space.to_string());
    let buckets = [(INVITE_USER, user.as_str()), (INVITE_SPACE, space.as_str())];
    state.limiter.reserve(&buckets)?;
    if state.email.is_none() || state.email_disabled {
        return Ok(None);
    }
    let counted = email_limits::reserve(&state.db, s.space, s.user_id, Kind::Invite).await;
    if counted.is_err() {
        state.limiter.refund(&buckets);
    }
    counted.map(Some)
}

/// The invitation was refused after [`invite_limits`] (forbidden, unknown,
/// a failed write): nothing is mailed, so nothing stays counted.
async fn give_back(state: &AppState, s: Scope, reserved: Option<i64>) {
    let (user, space) = (s.user_id.to_string(), s.space.to_string());
    state
        .limiter
        .refund(&[(INVITE_USER, user.as_str()), (INVITE_SPACE, space.as_str())]);
    if let Some(id) = reserved
        && let Err(e) = email_limits::release(&state.db, id).await
    {
        tracing::warn!(error = %e, "invitation e-mail reservation not given back");
    }
}

/// Nothing was written: 403 when the invitation is live (an owner
/// invitation the caller may not touch), else 404.
async fn refused(state: &AppState, s: Scope, id: Uuid) -> AppError {
    match invites::find(&state.db, s.space, id).await {
        Ok(Some(_)) => AppError::Forbidden,
        Ok(None) => AppError::NotFound,
        Err(e) => e,
    }
}

/// Mail the link when SMTP is configured — then it is never returned: the
/// inviter must not be able to accept in the invitee's name (create a
/// verified account for an address they do not own, or learn whether it has
/// an account). Without SMTP the link is returned for the inviter to pass on.
fn sent(
    state: &AppState,
    authed: &Authed,
    row: InviteRow,
    token: &str,
    locale: Option<&str>,
) -> Result<SentInvite, AppError> {
    let space = authed.space.as_ref().ok_or(AppError::NotFound)?;
    let url = format!(
        "{}/invite?token={token}",
        state.public.space_url(&space.slug)
    );
    if state.email.is_none() {
        return Ok(SentInvite {
            invite: row.into(),
            url: Some(url),
            email_sent: false,
            email_disabled: false,
        });
    }
    let email_sent = mail::send(
        state,
        &row.email,
        Mail::Invite {
            url,
            space: space.name.clone(),
            inviter: authed.user.display_name.clone(),
            role: row.role,
        },
        mail::locale(locale),
    );
    Ok(SentInvite {
        invite: row.into(),
        url: None,
        email_sent,
        email_disabled: state.email_disabled,
    })
}

async fn validate(
    state: &AppState,
    s: Scope,
    input: &InviteInput,
) -> Result<(String, Role), AppError> {
    let mut e = FieldErrors::new();
    let email = e.check("email", crate::auth::handlers::email(&input.email));
    let role = e.check("role", Role::from_input(input.role.as_deref()));
    if let Some(r) = role {
        e.check(
            "role",
            rules::can_invite(s.role, None, r).map_err(|_| "too_high"),
        );
    }
    if let Some(addr) = &email
        && let Some(user) = users::find_by_email(&state.db, addr).await?
        && space_repo::member_role(&state.db, s.space, user.id)
            .await?
            .is_some()
    {
        e.add("email", "already_member");
    }
    e.into_result()?;
    Ok((email.unwrap_or_default(), role.unwrap_or(Role::Accountant)))
}

#[utoipa::path(
    post,
    path = "/api/invites",
    tag = "members",
    security(("cookie" = []), ("bearer" = [])),
    request_body = InviteInput,
    responses(
        (status = 201, body = SentInvite),
        (status = 403, description = "`forbidden`: below admin, or replacing an owner invitation as an admin", body = ErrorBody),
        (status = 422, description = "`email: required | invalid | already_member`, `role: required | invalid | too_high`", body = ErrorBody),
        (status = 429, description = "`rate_limited`: 20 invitations / h per user, 50 / h per space; with SMTP also the space's and the user's e-mail limits", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    access: Manage,
    authed: Authed,
    ApiJson(input): ApiJson<InviteInput>,
) -> Result<(StatusCode, Json<SentInvite>), AppError> {
    let s = access.scope;
    let (email, role) = validate(&state, s, &input).await?;
    let reserved = invite_limits(&state, s).await?;
    let owner_ok = rules::may_touch_owner_invite(s.role);
    let (row, token) =
        match invites::upsert(&state.db, s.space, &email, role, s.user_id, owner_ok).await {
            Ok(Some(written)) => written,
            refused => {
                give_back(&state, s, reserved).await;
                return Err(refused.err().unwrap_or(AppError::Forbidden));
            }
        };
    let body = sent(&state, &authed, row, &token, input.locale.as_deref())?;
    Ok((StatusCode::CREATED, Json(body)))
}

/// A new token and 7-day expiry; the e-mail is sent again.
#[utoipa::path(
    post,
    path = "/api/invites/{id}/resend",
    tag = "members",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body(content = ResendInput, description = "Optional"),
    responses(
        (status = 200, body = SentInvite),
        (status = 403, description = "`forbidden`: below admin, or an owner invitation as an admin", body = ErrorBody),
        (status = 404, description = "Unknown, foreign or expired", body = ErrorBody),
        (status = 429, description = "`rate_limited`: 20 invitations / h per user, 50 / h per space; with SMTP also the space's and the user's e-mail limits", body = ErrorBody),
    )
)]
pub async fn resend(
    State(state): State<AppState>,
    access: Manage,
    authed: Authed,
    ApiPath(id): ApiPath<Uuid>,
    body: Bytes,
) -> Result<Json<SentInvite>, AppError> {
    let s = access.scope;
    let input: ResendInput = optional_json(&body)?;
    let reserved = invite_limits(&state, s).await?;
    let owner_ok = rules::may_touch_owner_invite(s.role);
    let (row, token) = match invites::renew(&state.db, s.space, id, s.user_id, owner_ok).await {
        Ok(Some(written)) => written,
        not_renewed => {
            give_back(&state, s, reserved).await;
            return Err(match not_renewed {
                Err(e) => e,
                Ok(_) => refused(&state, s, id).await,
            });
        }
    };
    Ok(Json(sent(
        &state,
        &authed,
        row,
        &token,
        input.locale.as_deref(),
    )?))
}

#[utoipa::path(
    delete,
    path = "/api/invites/{id}",
    tag = "members",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204, description = "Revoked; the link stops working"),
        (status = 403, description = "`forbidden`: below admin, or an owner invitation as an admin", body = ErrorBody),
        (status = 404, description = "Unknown or foreign", body = ErrorBody),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    access: Manage,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    let s = access.scope;
    let owner_ok = rules::may_touch_owner_invite(s.role);
    if !invites::delete(&state.db, s.space, id, owner_ok).await? {
        return Err(refused(&state, s, id).await);
    }
    Ok(StatusCode::NO_CONTENT)
}
