//! TOTP columns of `users`, `recovery_codes` and `mfa_logins`. Every state
//! change is one conditional statement (setup vs. enable, replay guard,
//! single-use codes and pending logins), so concurrent requests cannot both
//! win.

use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::sea_query::{Expr, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    FromQueryResult, PaginatorTrait, QueryFilter, QuerySelect, Set, Statement, TransactionTrait,
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::crypto;
use crate::auth::entity::{api_token, mfa_login, recovery_code, session, user_totp as user};
use crate::auth::session::Method;
use crate::error::AppError;
use crate::space::SpaceId;
use crate::space::entity::space;

/// How long a setup's secret waits for its first code.
pub const PENDING_SETUP: Duration = Duration::minutes(10);
/// How long a password-verified login waits for its code.
pub const PENDING_LOGIN: Duration = Duration::minutes(5);
/// Wrong codes after which a pending login dies.
pub const LOGIN_FAILURES: i32 = 5;

/// The user's TOTP columns (read where a code is checked or enrolment runs).
pub async fn totp(db: &impl ConnectionTrait, user_id: Uuid) -> Result<user::Model, AppError> {
    user::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or(AppError::Unauthorized)
}

/// Has the user TOTP on?
pub async fn mfa_enabled(db: &impl ConnectionTrait, user_id: Uuid) -> Result<bool, AppError> {
    Ok(totp(db, user_id).await?.totp_secret.is_some())
}

/// Store a setup's sealed secret (replacing an earlier one); `false` when
/// TOTP got enabled meanwhile.
pub async fn set_pending(
    db: &DatabaseConnection,
    user_id: Uuid,
    sealed: Vec<u8>,
) -> Result<bool, AppError> {
    let expires: DateTime<FixedOffset> = (Utc::now() + PENDING_SETUP).into();
    let res = user::Entity::update_many()
        .col_expr(user::Column::TotpPending, Expr::value(sealed))
        .col_expr(user::Column::TotpPendingExpiresAt, Expr::value(expires))
        .filter(user::Column::Id.eq(user_id))
        .filter(user::Column::TotpSecret.is_null())
        .exec(db)
        .await?;
    Ok(res.rows_affected == 1)
}

/// The pending secret becomes active (`last_step` = the step its first code
/// used) with fresh recovery codes; `false` when that pending secret is
/// gone, expired or replaced meanwhile, or TOTP is already on.
pub async fn activate(
    db: &DatabaseConnection,
    user_id: Uuid,
    pending: Vec<u8>,
    step: i64,
    code_hashes: Vec<String>,
) -> Result<bool, AppError> {
    let txn = db.begin().await?;
    let res = user::Entity::update_many()
        .col_expr(
            user::Column::TotpSecret,
            Expr::col(user::Column::TotpPending).into(),
        )
        .col_expr(user::Column::TotpPending, Expr::value(None::<Vec<u8>>))
        .col_expr(
            user::Column::TotpPendingExpiresAt,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(user::Column::TotpLastStep, Expr::value(step))
        .filter(user::Column::Id.eq(user_id))
        .filter(user::Column::TotpSecret.is_null())
        .filter(user::Column::TotpPending.eq(pending))
        .filter(user::Column::TotpPendingExpiresAt.gt(Utc::now()))
        .exec(&txn)
        .await?;
    if res.rows_affected != 1 {
        return Ok(false);
    }
    replace_recovery(&txn, user_id, code_hashes).await?;
    txn.commit().await?;
    Ok(true)
}

/// Accept `step` for the user (replay guard): `false` when a step at or
/// after it was already used, or TOTP is off.
pub async fn advance_step(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    step: i64,
) -> Result<bool, AppError> {
    let res = user::Entity::update_many()
        .col_expr(user::Column::TotpLastStep, Expr::value(step))
        .filter(user::Column::Id.eq(user_id))
        .filter(user::Column::TotpSecret.is_not_null())
        .filter(
            user::Column::TotpLastStep
                .is_null()
                .or(user::Column::TotpLastStep.lt(step)),
        )
        .exec(db)
        .await?;
    Ok(res.rows_affected == 1)
}

/// Spend one recovery code; `false` when unknown or used.
pub async fn use_recovery(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    hash: String,
) -> Result<bool, AppError> {
    let res = recovery_code::Entity::delete_many()
        .filter(recovery_code::Column::UserId.eq(user_id))
        .filter(recovery_code::Column::CodeHash.eq(hash))
        .exec(db)
        .await?;
    Ok(res.rows_affected == 1)
}

/// Regenerate: the user's codes become `code_hashes` (one transaction).
pub async fn set_recovery(
    db: &DatabaseConnection,
    user_id: Uuid,
    code_hashes: Vec<String>,
) -> Result<(), AppError> {
    let txn = db.begin().await?;
    replace_recovery(&txn, user_id, code_hashes).await?;
    txn.commit().await?;
    Ok(())
}

async fn replace_recovery(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    code_hashes: Vec<String>,
) -> Result<(), AppError> {
    recovery_code::Entity::delete_many()
        .filter(recovery_code::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    if code_hashes.is_empty() {
        return Ok(());
    }
    let now: DateTime<FixedOffset> = Utc::now().into();
    recovery_code::Entity::insert_many(code_hashes.into_iter().map(|hash| {
        recovery_code::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(user_id),
            code_hash: Set(hash),
            created_at: Set(now),
        }
    }))
    .exec(db)
    .await?;
    Ok(())
}

pub async fn recovery_left(db: &DatabaseConnection, user_id: Uuid) -> Result<u64, AppError> {
    Ok(recovery_code::Entity::find()
        .filter(recovery_code::Column::UserId.eq(user_id))
        .count(db)
        .await?)
}

#[derive(Debug, Clone, FromQueryResult, Serialize, ToSchema)]
pub struct SpaceRef {
    pub slug: String,
    pub name: String,
}

/// The user's spaces with the policy on, by name.
pub async fn required_by(
    db: &DatabaseConnection,
    user_id: Uuid,
) -> Result<Vec<SpaceRef>, AppError> {
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT s.slug, s.name FROM spaces s JOIN space_members m ON m.space_id = s.id \
         WHERE m.user_id = $1 AND s.require_mfa ORDER BY lower(s.name), s.slug",
        vec![user_id.into()],
    );
    Ok(SpaceRef::find_by_statement(stmt).all(db).await?)
}

/// Turn TOTP off: secret, pending setup, recovery codes and pending logins
/// go; so do the user's sessions and API tokens in spaces that require
/// TOTP (they would not be allowed to sign in there again).
pub async fn disable(db: &DatabaseConnection, user_id: Uuid) -> Result<(), AppError> {
    let txn = db.begin().await?;
    user::Entity::update_many()
        .col_expr(user::Column::TotpSecret, Expr::value(None::<Vec<u8>>))
        .col_expr(user::Column::TotpPending, Expr::value(None::<Vec<u8>>))
        .col_expr(
            user::Column::TotpPendingExpiresAt,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(user::Column::TotpLastStep, Expr::value(None::<i64>))
        .filter(user::Column::Id.eq(user_id))
        .exec(&txn)
        .await?;
    replace_recovery(&txn, user_id, Vec::new()).await?;
    mfa_login::Entity::delete_many()
        .filter(mfa_login::Column::UserId.eq(user_id))
        .exec(&txn)
        .await?;
    let requiring = || {
        Query::select()
            .column(space::Column::Id)
            .from(space::Entity)
            .and_where(space::Column::RequireMfa.eq(true))
            .to_owned()
    };
    session::Entity::delete_many()
        .filter(session::Column::UserId.eq(user_id))
        .filter(session::Column::SpaceId.in_subquery(requiring()))
        .exec(&txn)
        .await?;
    api_token::Entity::delete_many()
        .filter(api_token::Column::UserId.eq(user_id))
        .filter(api_token::Column::SpaceId.in_subquery(requiring()))
        .exec(&txn)
        .await?;
    txn.commit().await?;
    Ok(())
}

/// What a pending login becomes once its code is right.
#[derive(Debug, Clone)]
pub struct Pending {
    /// What the session records.
    pub method: Method,
    /// A Google invitation accept: sha256 of the invitation token (the
    /// membership is created together with the session).
    pub invite_hash: Option<String>,
}

impl Pending {
    pub fn login(method: Method) -> Self {
        Self {
            method,
            invite_hash: None,
        }
    }
}

/// Start a pending login; returns the cookie value. Expired rows of
/// anyone are purged on the way.
pub async fn start_login(
    db: &DatabaseConnection,
    user_id: Uuid,
    space: Option<SpaceId>,
    user_agent: Option<String>,
    pending: Pending,
) -> Result<String, AppError> {
    mfa_login::Entity::delete_many()
        .filter(mfa_login::Column::ExpiresAt.lte(Utc::now()))
        .exec(db)
        .await?;
    let value = crypto::random_token()?;
    let now: DateTime<FixedOffset> = Utc::now().into();
    mfa_login::ActiveModel {
        id: Set(Uuid::new_v4()),
        token_hash: Set(crypto::digest(&value)),
        user_id: Set(user_id),
        space_id: Set(space.map(SpaceId::uuid)),
        user_agent: Set(user_agent),
        failures: Set(0),
        created_at: Set(now),
        expires_at: Set(now + PENDING_LOGIN),
        login_method: Set(pending.method.as_str().to_string()),
        invite_hash: Set(pending.invite_hash),
    }
    .insert(db)
    .await?;
    Ok(value)
}

/// The live pending login of `cookie` on this host, row-locked until the
/// end of `txn`: concurrent code attempts on one pending login run one
/// after the other (the failure cap and single use hold).
pub async fn lock_login(
    txn: &impl ConnectionTrait,
    cookie: &str,
    space: Option<SpaceId>,
) -> Result<Option<mfa_login::Model>, AppError> {
    let row = mfa_login::Entity::find()
        .filter(mfa_login::Column::TokenHash.eq(crypto::digest(cookie)))
        .filter(mfa_login::Column::ExpiresAt.gt(Utc::now()))
        .filter(mfa_login::Column::Failures.lt(LOGIN_FAILURES))
        .lock_exclusive()
        .one(txn)
        .await?;
    Ok(row.filter(|r| r.space_id == space.map(SpaceId::uuid)))
}

/// Count a wrong code on the locked row; at [`LOGIN_FAILURES`] the pending
/// login is deleted.
pub async fn fail_login(
    txn: &impl ConnectionTrait,
    row: &mfa_login::Model,
) -> Result<(), AppError> {
    if row.failures + 1 >= LOGIN_FAILURES {
        return take_login(txn, row.id).await.map(|_| ());
    }
    mfa_login::Entity::update_many()
        .col_expr(
            mfa_login::Column::Failures,
            Expr::col(mfa_login::Column::Failures).add(1),
        )
        .filter(mfa_login::Column::Id.eq(row.id))
        .exec(txn)
        .await?;
    Ok(())
}

/// Consume the pending login (single use).
pub async fn take_login(db: &impl ConnectionTrait, id: Uuid) -> Result<bool, AppError> {
    let res = mfa_login::Entity::delete_many()
        .filter(mfa_login::Column::Id.eq(id))
        .exec(db)
        .await?;
    Ok(res.rows_affected == 1)
}
