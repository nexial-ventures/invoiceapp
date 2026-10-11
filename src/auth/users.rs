//! Users and their single-use e-mail tokens (verification 24 h, password
//! reset 1 h; sha256 stored).

use chrono::{Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, Statement, TransactionTrait,
};
use uuid::Uuid;

use super::crypto;
use super::entity::{user, user_token};
use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Verify,
    Reset,
    /// `POST /api/account/google/link` → `start` (single use, 5 min).
    GoogleLink,
}

impl TokenKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
            Self::GoogleLink => "google_link",
        }
    }

    fn lifetime(self) -> Duration {
        match self {
            Self::Verify => Duration::hours(24),
            Self::Reset => Duration::hours(1),
            Self::GoogleLink => Duration::minutes(5),
        }
    }
}

/// E-mails are stored trimmed + lowercased.
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

pub async fn find(db: &impl ConnectionTrait, id: Uuid) -> Result<Option<user::Model>, AppError> {
    Ok(user::Entity::find_by_id(id).one(db).await?)
}

pub async fn find_by_email(
    db: &impl ConnectionTrait,
    email: &str,
) -> Result<Option<user::Model>, AppError> {
    Ok(user::Entity::find()
        .filter(user::Column::Email.eq(normalize_email(email)))
        .one(db)
        .await?)
}

/// A new unverified user (`password_hash` `None`: an account created
/// through Google); `None` when the e-mail is taken (a concurrent
/// registration lost the race).
pub async fn create(
    db: &impl ConnectionTrait,
    email: &str,
    display_name: &str,
    password_hash: Option<String>,
) -> Result<Option<user::Model>, AppError> {
    let row = user::ActiveModel {
        id: Set(Uuid::new_v4()),
        email: Set(normalize_email(email)),
        display_name: Set(display_name.to_string()),
        password_hash: Set(password_hash),
        email_verified_at: Set(None),
        disabled: Set(false),
        created_at: Set(Utc::now().into()),
        google_sub: Set(None),
    }
    .insert(db)
    .await;
    match row {
        Ok(u) => Ok(Some(u)),
        Err(e) if crate::error::unique_violation(&e).as_deref() == Some("users_email_key") => {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn set_password(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    password_hash: String,
) -> Result<(), AppError> {
    user::Entity::update_many()
        .col_expr(user::Column::PasswordHash, Expr::value(password_hash))
        .filter(user::Column::Id.eq(user_id))
        .exec(db)
        .await?;
    Ok(())
}

/// Mark the e-mail verified (verify link, password reset, invitation,
/// Google). A second factor or Google account set up while the address was
/// unproven is dropped — TOTP secret, pending setup, recovery codes, pending
/// logins, the Google link: whoever set them up was not shown to own the
/// address. A no-op for an already verified user.
pub async fn mark_verified(db: &impl ConnectionTrait, user_id: Uuid) -> Result<(), AppError> {
    verify(db, user_id, false).await
}

/// [`mark_verified`] for a user who has just proved their TOTP with a code
/// in the same transaction (an invitation accept): that second factor is
/// theirs and stays; only the Google link set up while unverified goes.
pub async fn mark_verified_keeping_mfa(
    db: &impl ConnectionTrait,
    user_id: Uuid,
) -> Result<(), AppError> {
    verify(db, user_id, true).await
}

async fn verify(db: &impl ConnectionTrait, user_id: Uuid, keep_mfa: bool) -> Result<(), AppError> {
    let stmt = |sql: &str| {
        Statement::from_sql_and_values(db.get_database_backend(), sql, [user_id.into()])
    };
    let sql = if keep_mfa {
        "UPDATE users SET email_verified_at = now(), google_sub = NULL \
         WHERE id = $1 AND email_verified_at IS NULL"
    } else {
        "UPDATE users SET email_verified_at = now(), google_sub = NULL, totp_secret = NULL, \
         totp_pending = NULL, totp_pending_expires_at = NULL, totp_last_step = NULL \
         WHERE id = $1 AND email_verified_at IS NULL"
    };
    let res = db.execute(stmt(sql)).await?;
    if res.rows_affected() == 1 && !keep_mfa {
        db.execute(stmt("DELETE FROM recovery_codes WHERE user_id = $1"))
            .await?;
        db.execute(stmt("DELETE FROM mfa_logins WHERE user_id = $1"))
            .await?;
    }
    Ok(())
}

/// A fresh single-use token of `kind` for the user; returns the raw value
/// (it only ever travels in the e-mail).
pub async fn issue_token(
    db: &DatabaseConnection,
    user_id: Uuid,
    kind: TokenKind,
) -> Result<String, AppError> {
    let raw = crypto::random_token()?;
    let now = Utc::now();
    user_token::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(user_id),
        kind: Set(kind.as_str().to_string()),
        token_hash: Set(crypto::digest(&raw)),
        created_at: Set(now.into()),
        expires_at: Set((now + kind.lifetime()).into()),
        used_at: Set(None),
    }
    .insert(db)
    .await?;
    Ok(raw)
}

/// Consume a token of `kind` inside `f`'s transaction: marks it used and
/// runs `f(user_id)`. Unknown, expired, used or of the other kind → 422
/// `token: invalid`.
pub async fn consume_token<F, Fut>(
    db: &DatabaseConnection,
    raw: &str,
    kind: TokenKind,
    f: F,
) -> Result<(), AppError>
where
    F: FnOnce(sea_orm::DatabaseTransaction, Uuid) -> Fut + Send,
    Fut: std::future::Future<Output = Result<sea_orm::DatabaseTransaction, AppError>> + Send,
{
    let txn = db.begin().await?;
    let used = user_token::Entity::update_many()
        .col_expr(user_token::Column::UsedAt, Expr::value(Utc::now()))
        .filter(user_token::Column::TokenHash.eq(crypto::digest(raw)))
        .filter(user_token::Column::Kind.eq(kind.as_str()))
        .filter(user_token::Column::UsedAt.is_null())
        .filter(user_token::Column::ExpiresAt.gt(Utc::now()))
        .exec_with_returning(&txn)
        .await?;
    let Some(token) = used.into_iter().next() else {
        return Err(AppError::field("token", "invalid"));
    };
    let txn = f(txn, token.user_id).await?;
    txn.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_normalized() {
        assert_eq!(
            normalize_email("  Jan.Novak@Example.COM "),
            "jan.novak@example.com"
        );
    }

    #[test]
    fn token_lifetimes() {
        assert_eq!(TokenKind::Verify.lifetime(), Duration::hours(24));
        assert_eq!(TokenKind::Reset.lifetime(), Duration::hours(1));
        assert_eq!(TokenKind::GoogleLink.lifetime(), Duration::minutes(5));
    }
}
