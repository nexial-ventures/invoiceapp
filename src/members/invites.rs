//! Pending invitations: one per (space, e-mail), the token in the link
//! stored as sha256, valid 7 days, single use. Expired rows are never
//! served and are purged when the space's list is read.

use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    FromQueryResult, QueryFilter, Statement,
};
use uuid::Uuid;

use super::entity::space_invite::{self, Column, Entity};
use crate::auth::crypto;
use crate::error::AppError;
use crate::space::{Role, SpaceId};

pub const LIFETIME: Duration = Duration::days(7);

#[derive(Debug, Clone, FromQueryResult)]
struct Row {
    id: Uuid,
    email: String,
    role: String,
    created_at: DateTime<FixedOffset>,
    expires_at: DateTime<FixedOffset>,
    inviter_email: String,
    inviter_name: String,
}

#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: Uuid,
    pub email: String,
    pub role: Role,
    pub created_at: DateTime<FixedOffset>,
    pub expires_at: DateTime<FixedOffset>,
    pub inviter_email: String,
    pub inviter_name: String,
}

/// The item columns of an invitation `i` joined with its inviter `u`.
const COLS: &str = "i.id, i.email, i.role, i.created_at, i.expires_at, \
     u.email AS inviter_email, u.display_name AS inviter_name";

async fn rows(
    db: &impl ConnectionTrait,
    sql: String,
    values: Vec<sea_orm::Value>,
) -> Result<Vec<InviteRow>, AppError> {
    let stmt = Statement::from_sql_and_values(db.get_database_backend(), sql, values);
    let rows = Row::find_by_statement(stmt).all(db).await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(InviteRow {
                role: Role::parse(&r.role)?,
                id: r.id,
                email: r.email,
                created_at: r.created_at,
                expires_at: r.expires_at,
                inviter_email: r.inviter_email,
                inviter_name: r.inviter_name,
            })
        })
        .collect())
}

/// Live invitations of `space` (only `id` when given), newest first.
async fn query(
    db: &impl ConnectionTrait,
    space: SpaceId,
    id: Option<Uuid>,
) -> Result<Vec<InviteRow>, AppError> {
    let sql = format!(
        "SELECT {COLS} FROM space_invites i JOIN users u ON u.id = i.invited_by \
         WHERE i.space_id = $1 AND i.expires_at > now() AND ($2::uuid IS NULL OR i.id = $2) \
         ORDER BY i.created_at DESC, i.id"
    );
    rows(db, sql, vec![space.into(), id.into()]).await
}

/// Purge the expired rows of `space`, then list the live ones.
pub async fn list(db: &DatabaseConnection, space: SpaceId) -> Result<Vec<InviteRow>, AppError> {
    Entity::delete_many()
        .filter(Column::SpaceId.eq(space.uuid()))
        .filter(Column::ExpiresAt.lte(Utc::now()))
        .exec(db)
        .await?;
    query(db, space, None).await
}

pub async fn find(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
) -> Result<Option<InviteRow>, AppError> {
    Ok(query(db, space, Some(id)).await?.into_iter().next())
}

/// Invite `email` with `role`, replacing any invitation for it (the row
/// keeps its id; new role, inviter, token, creation and expiry) — in one
/// statement. A live owner invitation is replaced only when `owner_ok`;
/// otherwise `None` (nothing written). Returns the item and the token (it
/// only ever travels in the link).
pub async fn upsert(
    db: &DatabaseConnection,
    space: SpaceId,
    email: &str,
    role: Role,
    invited_by: Uuid,
    owner_ok: bool,
) -> Result<Option<(InviteRow, String)>, AppError> {
    let token = crypto::random_token()?;
    let now = Utc::now();
    let sql = format!(
        "WITH i AS (INSERT INTO space_invites \
         (id, space_id, email, role, token_hash, invited_by, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT ON CONSTRAINT space_invites_space_email_key DO UPDATE SET \
         role = EXCLUDED.role, token_hash = EXCLUDED.token_hash, invited_by = EXCLUDED.invited_by, \
         created_at = EXCLUDED.created_at, expires_at = EXCLUDED.expires_at \
         WHERE $9 OR space_invites.role <> 'owner' OR space_invites.expires_at <= now() \
         RETURNING *) \
         SELECT {COLS} FROM i JOIN users u ON u.id = i.invited_by"
    );
    let values = vec![
        Uuid::new_v4().into(),
        space.into(),
        email.into(),
        role.as_str().into(),
        crypto::digest(&token).into(),
        invited_by.into(),
        now.into(),
        (now + LIFETIME).into(),
        owner_ok.into(),
    ];
    Ok(rows(db, sql, values).await?.pop().map(|r| (r, token)))
}

/// A new token and a new 7-day expiry for the live invitation `id`
/// (`invited_by` = who sends it now) in one statement; an owner invitation
/// only when `owner_ok`. `None` when nothing matched.
pub async fn renew(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    invited_by: Uuid,
    owner_ok: bool,
) -> Result<Option<(InviteRow, String)>, AppError> {
    let token = crypto::random_token()?;
    let sql = format!(
        "WITH i AS (UPDATE space_invites SET token_hash = $1, expires_at = $2, invited_by = $3 \
         WHERE id = $4 AND space_id = $5 AND expires_at > now() AND ($6 OR role <> 'owner') \
         RETURNING *) \
         SELECT {COLS} FROM i JOIN users u ON u.id = i.invited_by"
    );
    let values = vec![
        crypto::digest(&token).into(),
        (Utc::now() + LIFETIME).into(),
        invited_by.into(),
        id.into(),
        space.into(),
        owner_ok.into(),
    ];
    Ok(rows(db, sql, values).await?.pop().map(|r| (r, token)))
}

/// Revoke invitation `id` of `space` in one statement; a live owner
/// invitation only when `owner_ok`. `false` when nothing matched.
pub async fn delete(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    owner_ok: bool,
) -> Result<bool, AppError> {
    let res = db
        .execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "DELETE FROM space_invites WHERE id = $1 AND space_id = $2 \
             AND ($3 OR role <> 'owner' OR expires_at <= now())",
            vec![id.into(), space.into(), owner_ok.into()],
        ))
        .await?;
    Ok(res.rows_affected() > 0)
}

/// The live invitation of `space` behind `token`.
pub async fn by_token(
    db: &impl ConnectionTrait,
    space: SpaceId,
    token: &str,
) -> Result<Option<space_invite::Model>, AppError> {
    if token.is_empty() {
        return Ok(None);
    }
    by_hash(db, space, &crypto::digest(token)).await
}

/// The live invitation of `space` whose token has sha256 `token_hash`.
pub async fn by_hash(
    db: &impl ConnectionTrait,
    space: SpaceId,
    token_hash: &str,
) -> Result<Option<space_invite::Model>, AppError> {
    Ok(Entity::find()
        .filter(Column::TokenHash.eq(token_hash))
        .filter(Column::SpaceId.eq(space.uuid()))
        .filter(Column::ExpiresAt.gt(Utc::now()))
        .one(db)
        .await?)
}

/// Delete (= use up) the live invitation whose token has sha256
/// `token_hash`; `None` when it is gone meanwhile. The delete row-locks it:
/// one of two parallel accepts wins.
pub async fn consume(
    txn: &DatabaseTransaction,
    space: SpaceId,
    token_hash: &str,
) -> Result<Option<space_invite::Model>, AppError> {
    let gone = Entity::delete_many()
        .filter(Column::TokenHash.eq(token_hash))
        .filter(Column::SpaceId.eq(space.uuid()))
        .filter(Column::ExpiresAt.gt(Utc::now()))
        .exec_with_returning(txn)
        .await?;
    Ok(gone.into_iter().next())
}
