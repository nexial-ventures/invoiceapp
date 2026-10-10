//! Google accounts on the user side: resolving the user of a verified
//! identity (the callback), attaching / detaching the `sub`, and
//! `DELETE /api/account/google`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Statement,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::Fail;
use super::idtoken::Identity;
use crate::app::AppState;
use crate::auth::ctx::Authed;
use crate::auth::entity::user;
use crate::auth::handlers::check_password;
use crate::auth::host::HostCtx;
use crate::auth::users::TokenKind;
use crate::auth::{mfa, session, users};
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::ApiJson;

const NAME_MAX: usize = 100;

pub async fn find_by_sub(
    db: &impl ConnectionTrait,
    sub: &str,
) -> Result<Option<user::Model>, AppError> {
    Ok(user::Entity::find()
        .filter(user::Column::GoogleSub.eq(sub))
        .one(db)
        .await?)
}

/// Attach `sub` to the user unless another Google account is attached
/// already; `false` when it is (or `sub` belongs to someone else).
pub async fn attach(db: &impl ConnectionTrait, user_id: Uuid, sub: &str) -> Result<bool, AppError> {
    let res = user::Entity::update_many()
        .col_expr(user::Column::GoogleSub, Expr::value(sub))
        .filter(user::Column::Id.eq(user_id))
        .filter(
            user::Column::GoogleSub
                .is_null()
                .or(user::Column::GoogleSub.eq(sub)),
        )
        .exec(db)
        .await;
    match res {
        Ok(r) => Ok(r.rows_affected == 1),
        Err(e) if crate::error::unique_violation(&e).as_deref() == Some("users_google_sub_key") => {
            Ok(false)
        }
        Err(e) => Err(e.into()),
    }
}

/// An **unverified** account with the Google e-mail: Google's verified
/// address proves the owner, whoever registered it. Whatever the registrant
/// set up without proving the address goes — password, TOTP, recovery
/// codes, sessions, pending logins — then the account is verified and
/// linked — replacing a Google account linked while the address was
/// unproven (one transaction). `false` when it got verified meanwhile.
async fn claim_unverified(
    db: &DatabaseConnection,
    user_id: Uuid,
    sub: &str,
) -> Result<bool, AppError> {
    let txn = db.begin().await?;
    let res = txn
        .execute(Statement::from_sql_and_values(
            txn.get_database_backend(),
            "UPDATE users SET password_hash = NULL, totp_secret = NULL, totp_pending = NULL, \
             totp_pending_expires_at = NULL, totp_last_step = NULL, email_verified_at = now(), \
             google_sub = $2 WHERE id = $1 AND email_verified_at IS NULL",
            vec![user_id.into(), sub.into()],
        ))
        .await?;
    if res.rows_affected() != 1 {
        return Ok(false);
    }
    txn.execute(Statement::from_sql_and_values(
        txn.get_database_backend(),
        "DELETE FROM recovery_codes WHERE user_id = $1",
        vec![user_id.into()],
    ))
    .await?;
    session::delete_others(&txn, user_id, None).await?;
    txn.commit().await?;
    Ok(true)
}

/// A new verified account without a password, linked to `id`.
async fn create(db: &DatabaseConnection, id: &Identity) -> Result<Option<Uuid>, AppError> {
    let local = id.email.split('@').next().unwrap_or_default();
    let name: String = id
        .name
        .as_deref()
        .unwrap_or(local)
        .chars()
        .filter(|c| !c.is_control())
        .take(NAME_MAX)
        .collect();
    let name = if name.trim().is_empty() {
        "Google"
    } else {
        name.trim()
    };
    let txn = db.begin().await?;
    let Some(u) = users::create(&txn, &id.email, name, None).await? else {
        return Ok(None);
    };
    users::mark_verified(&txn, u.id).await?;
    if !attach(&txn, u.id, &id.sub).await? {
        return Ok(None);
    }
    txn.commit().await?;
    Ok(Some(u.id))
}

/// Who a verified identity signs in as (`login`, `invite`): the user with
/// this `sub`; else the user with this e-mail, linked automatically; else a
/// new account when `may_create`. Disabled → `disabled`; the e-mail's
/// verified account linked to another Google account → `already_linked`
/// (an unverified one is claimed, see [`claim_unverified`]).
pub async fn resolve(
    db: &DatabaseConnection,
    id: &Identity,
    may_create: bool,
) -> Result<Uuid, Fail> {
    if let Some(u) = find_by_sub(db, &id.sub).await? {
        return if u.disabled {
            Err(Fail::Disabled)
        } else {
            Ok(u.id)
        };
    }
    let Some(u) = users::find_by_email(db, &id.email).await? else {
        if !may_create {
            return Err(Fail::NotAllowed);
        }
        // A parallel request created or linked it: start over once.
        return match create(db, id).await? {
            Some(uid) => Ok(uid),
            None => Box::pin(resolve(db, id, false)).await,
        };
    };
    if u.disabled {
        return Err(Fail::Disabled);
    }
    let linked = if u.email_verified_at.is_none() {
        claim_unverified(db, u.id, &id.sub).await?
    } else if u.google_sub.as_deref().is_some_and(|s| s != id.sub) {
        false
    } else {
        attach(db, u.id, &id.sub).await?
    };
    if linked {
        Ok(u.id)
    } else {
        Err(Fail::AlreadyLinked)
    }
}

/// `link`: the session's user may take `sub` unless it belongs to someone
/// else or the user has another Google account linked. Only a **verified**
/// user may link: an unproven registration of someone else's address must
/// not get a way to sign in that survives the owner claiming it.
pub async fn check_link(db: &DatabaseConnection, user_id: Uuid, sub: &str) -> Result<(), Fail> {
    if find_by_sub(db, sub).await?.is_some_and(|u| u.id != user_id) {
        return Err(Fail::AlreadyLinked);
    }
    match users::find(db, user_id).await? {
        Some(u) if u.disabled => Err(Fail::Disabled),
        Some(u) if u.email_verified_at.is_none() => Err(Fail::Invalid),
        Some(u) if u.google_sub.as_deref().is_some_and(|s| s != sub) => Err(Fail::AlreadyLinked),
        Some(_) => Ok(()),
        None => Err(Fail::Invalid),
    }
}

/// May an account be created for an identity nobody has (no user with its
/// `sub` or e-mail)? For an invitation of exactly this e-mail (`invited`,
/// checked by the caller), or a login on the base host while registration
/// is on.
pub fn may_create(invited: bool, base_host: bool, registration: bool) -> bool {
    invited || (base_host && registration)
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct ConfirmInput {
    pub password: String,
    /// Users with TOTP: a TOTP or recovery code.
    pub code: Option<String>,
}

/// Google on, a session, the own password and the step-up code.
async fn confirm(state: &AppState, authed: &Authed, input: ConfirmInput) -> Result<(), AppError> {
    state.google.as_ref().ok_or(AppError::NotFound)?;
    authed.require_session()?;
    let mut e = FieldErrors::new();
    if let Err(reason) = check_password(state, &authed.user, input.password).await? {
        e.add("password", reason);
    }
    let code = input.code.as_deref();
    mfa::step_up(state, &authed.user, authed.mfa_enabled, code, &mut e).await?;
    e.into_result()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkStart {
    /// `…/api/auth/google/start?purpose=link&grant=…` on this host — navigate
    /// to it (single use, 5 min).
    pub url: String,
}

/// Start linking a Google account: re-confirm with the password (and the
/// code), get the URL that starts the flow.
#[utoipa::path(
    post,
    path = "/api/account/google/link",
    tag = "auth",
    security(("cookie" = [])),
    request_body = ConfirmInput,
    responses(
        (status = 200, body = LinkStart),
        (status = 403, description = "`forbidden` with a token, `email_unverified`", body = ErrorBody),
        (status = 404, description = "Sign in with Google is not configured", body = ErrorBody),
        (status = 422, description = "`password: invalid | not_set`, `code: required | invalid` (users with TOTP)", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket)", body = ErrorBody),
    )
)]
pub async fn link(
    State(state): State<AppState>,
    Extension(host): Extension<HostCtx>,
    authed: Authed,
    ApiJson(input): ApiJson<ConfirmInput>,
) -> Result<Json<LinkStart>, AppError> {
    state.google.as_ref().ok_or(AppError::NotFound)?;
    authed.require_session()?;
    if authed.user.email_verified_at.is_none() {
        return Err(AppError::EmailUnverified);
    }
    confirm(&state, &authed, input).await?;
    let grant = users::issue_token(&state.db, authed.user.id, TokenKind::GoogleLink).await?;
    Ok(Json(LinkStart {
        url: format!(
            "{}/api/auth/google/start?purpose=link&grant={grant}",
            super::host_url(&state, &host)
        ),
    }))
}

/// Unlink the Google account. Needs a password set, so an account never
/// loses its last way to sign in.
#[utoipa::path(
    delete,
    path = "/api/account/google",
    tag = "auth",
    security(("cookie" = [])),
    request_body = ConfirmInput,
    responses(
        (status = 204, description = "Unlinked (also when nothing was linked)"),
        (status = 403, description = "`forbidden` with a token", body = ErrorBody),
        (status = 404, description = "Sign in with Google is not configured", body = ErrorBody),
        (status = 422, description = "`password: invalid | not_set`, `code: required | invalid` (users with TOTP)", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket)", body = ErrorBody),
    )
)]
pub async fn unlink(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(input): ApiJson<ConfirmInput>,
) -> Result<StatusCode, AppError> {
    confirm(&state, &authed, input).await?;
    user::Entity::update_many()
        .col_expr(user::Column::GoogleSub, Expr::value(None::<String>))
        .filter(user::Column::Id.eq(authed.user.id))
        // The password checked above must still be there.
        .filter(user::Column::PasswordHash.is_not_null())
        .exec(&state.db)
        .await?;
    tracing::info!(user = %authed.user.id, "Google account unlinked");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_may_get_a_new_account() {
        assert!(may_create(false, true, true));
        assert!(!may_create(false, false, true));
        assert!(!may_create(false, true, false));
        assert!(may_create(true, false, false));
    }
}
