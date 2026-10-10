//! A session cookie or API token → its row, the user and the membership in
//! the credential's space, in **one** joined query per request. The
//! `last_seen_at` / `last_used_at` write is a second query at most once a
//! minute per credential.

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    Statement,
};
use uuid::Uuid;

use super::entity::{api_token, session, user};
use super::{crypto, tokens};
use crate::error::AppError;
use crate::space::{Role, SpaceId};

const TOUCH_EVERY: Duration = Duration::minutes(1);

const USER_COLUMNS: &str = "u.id AS u_id, u.email, u.display_name, u.password_hash, \
     u.email_verified_at, u.disabled, u.created_at AS u_created, u.google_sub, \
     u.totp_secret IS NOT NULL AS mfa_enabled, m.role AS member_role";

#[derive(Debug, FromQueryResult)]
struct Row {
    cred_id: Uuid,
    cred_space: Option<Uuid>,
    /// Session: `last_seen_at`; token: `last_used_at`.
    touched: Option<DateTime<FixedOffset>>,
    created_at: DateTime<FixedOffset>,
    session_expires: Option<DateTime<FixedOffset>>,
    token_expires: Option<NaiveDate>,
    token_role: Option<String>,
    member_role: Option<String>,
    u_id: Uuid,
    email: String,
    display_name: String,
    password_hash: Option<String>,
    email_verified_at: Option<DateTime<FixedOffset>>,
    disabled: bool,
    u_created: DateTime<FixedOffset>,
    google_sub: Option<String>,
    mfa_enabled: bool,
}

/// A live credential with its user and membership.
#[derive(Debug, Clone)]
pub struct Found {
    pub id: Uuid,
    pub user: user::Model,
    /// The role in the credential's space (`None`: no membership, or the
    /// base host).
    pub member_role: Option<Role>,
    /// Tokens only.
    pub token_role: Option<Role>,
    /// The user has TOTP on.
    pub mfa_enabled: bool,
}

impl Row {
    fn found(self) -> Found {
        Found {
            id: self.cred_id,
            member_role: self.member_role.as_deref().and_then(Role::parse),
            token_role: self.token_role.as_deref().and_then(Role::parse),
            mfa_enabled: self.mfa_enabled,
            user: user::Model {
                id: self.u_id,
                email: self.email,
                display_name: self.display_name,
                password_hash: self.password_hash,
                email_verified_at: self.email_verified_at,
                disabled: self.disabled,
                created_at: self.u_created,
                google_sub: self.google_sub,
            },
        }
    }
}

async fn row(
    db: &DatabaseConnection,
    sql: String,
    values: Vec<sea_orm::Value>,
) -> Result<Option<Row>, AppError> {
    let stmt = Statement::from_sql_and_values(db.get_database_backend(), sql, values);
    Ok(Row::find_by_statement(stmt).one(db).await?)
}

/// Idle (14 days since last seen) or absolute (`expires_at`) expiry.
fn session_expired(
    seen: DateTime<Utc>,
    expires: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    expires.is_none_or(|e| now >= e) || now >= seen + super::session::IDLE
}

/// The live session of `cookie` on this host (`space` = the host's space,
/// `None` = base host). Expired sessions are deleted on sight.
pub async fn session(
    db: &DatabaseConnection,
    cookie: &str,
    space: Option<SpaceId>,
) -> Result<Option<Found>, AppError> {
    let sql = format!(
        "SELECT s.id AS cred_id, s.space_id AS cred_space, s.last_seen_at AS touched, \
         s.created_at, s.expires_at AS session_expires, NULL::date AS token_expires, \
         NULL::text AS token_role, {USER_COLUMNS} \
         FROM sessions s JOIN users u ON u.id = s.user_id \
         LEFT JOIN space_members m ON m.space_id = s.space_id AND m.user_id = s.user_id \
         WHERE s.token_hash = $1"
    );
    let Some(r) = row(db, sql, vec![crypto::digest(cookie).into()]).await? else {
        return Ok(None);
    };
    let now = Utc::now();
    let seen = r.touched.unwrap_or(r.created_at);
    if session_expired(seen.to_utc(), r.session_expires.map(|e| e.to_utc()), now) {
        session::Entity::delete_by_id(r.cred_id).exec(db).await?;
        return Ok(None);
    }
    if r.cred_space != space.map(SpaceId::uuid) {
        return Ok(None);
    }
    if now - seen.to_utc() >= TOUCH_EVERY {
        session::Entity::update_many()
            .col_expr(session::Column::LastSeenAt, Expr::value(now))
            .filter(session::Column::Id.eq(r.cred_id))
            .exec(db)
            .await?;
    }
    Ok(Some(r.found()))
}

/// The live API token `token` of `space`; unknown, foreign, expired → `None`.
pub async fn token(
    db: &DatabaseConnection,
    token: &str,
    space: SpaceId,
) -> Result<Option<Found>, AppError> {
    if !crypto::looks_like_api_token(token) {
        return Ok(None);
    }
    let sql = format!(
        "SELECT t.id AS cred_id, t.space_id AS cred_space, t.last_used_at AS touched, \
         t.created_at, NULL::timestamptz AS session_expires, t.expires_at AS token_expires, \
         t.role AS token_role, {USER_COLUMNS} \
         FROM api_tokens t JOIN users u ON u.id = t.user_id \
         LEFT JOIN space_members m ON m.space_id = t.space_id AND m.user_id = t.user_id \
         WHERE t.token_hash = $1 AND t.space_id = $2"
    );
    let Some(r) = row(db, sql, vec![crypto::digest(token).into(), space.into()]).await? else {
        return Ok(None);
    };
    let now = Utc::now();
    if tokens::repo::expired(r.token_expires, now.date_naive()) {
        return Ok(None);
    }
    if r.touched.is_none_or(|t| now - t.to_utc() >= TOUCH_EVERY) {
        api_token::Entity::update_many()
            .col_expr(api_token::Column::LastUsedAt, Expr::value(now))
            .filter(api_token::Column::Id.eq(r.cred_id))
            .exec(db)
            .await?;
    }
    Ok(Some(r.found()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::session::{ABSOLUTE, IDLE};

    #[test]
    fn session_expiry_rules() {
        let now = Utc::now();
        assert!(!session_expired(now, Some(now + ABSOLUTE), now));
        assert!(session_expired(now - IDLE, Some(now + ABSOLUTE), now));
        assert!(!session_expired(
            now - Duration::days(13),
            Some(now + Duration::days(60)),
            now
        ));
        assert!(session_expired(now, Some(now), now));
        assert!(session_expired(now, None, now));
    }
}
