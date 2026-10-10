//! Server-side sessions behind the `invoice_session` cookie: host-only, the
//! DB keeps sha256 of the cookie value. Idle expiry 14 days (sliding,
//! `last_seen_at` written at most once a minute), absolute 90 days. A
//! session is valid only on the host it was created on (base host or its
//! space).

use axum::http::{HeaderMap, header};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use super::crypto;
use super::entity::session::{ActiveModel, Column, Entity, Model};
use crate::error::AppError;
use crate::space::SpaceId;

pub const COOKIE: &str = "invoice_session";
pub const IDLE: Duration = Duration::days(14);
pub const ABSOLUTE: Duration = Duration::days(90);
const USER_AGENT_MAX: usize = 200;

/// How a session (or a pending TOTP login) was signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Password,
    Google,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Google => "google",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "password" => Some(Self::Password),
            "google" => Some(Self::Google),
            _ => None,
        }
    }
}

/// A host-only `Set-Cookie` (`HttpOnly`, `SameSite=Lax`, `Path=/`, `Secure`
/// on https); `max_age` 0 with an empty value removes the cookie.
pub fn cookie(name: &str, value: &str, max_age: Duration, secure: bool) -> String {
    format!(
        "{name}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{}",
        max_age.num_seconds(),
        if secure { "; Secure" } else { "" }
    )
}

/// `Set-Cookie` for a new session.
pub fn set_cookie(value: &str, secure: bool) -> String {
    cookie(COOKIE, value, ABSOLUTE, secure)
}

/// `Set-Cookie` that removes the cookie.
pub fn clear_cookie(secure: bool) -> String {
    cookie(COOKIE, "", Duration::zero(), secure)
}

/// The `invoice_session` value of the request, if any.
pub fn cookie_value(headers: &HeaderMap) -> Option<String> {
    named_cookie(headers, COOKIE)
}

/// The non-empty value of cookie `name` in the request, if any.
pub fn named_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|part| {
            part.trim()
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('='))
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
}

/// Trimmed to 200 chars (on a char boundary).
pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    let ua = headers.get(header::USER_AGENT)?.to_str().ok()?.trim();
    (!ua.is_empty()).then(|| ua.chars().take(USER_AGENT_MAX).collect())
}

/// Start a session; returns the cookie value (shown to nobody else).
pub async fn create(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    space: Option<SpaceId>,
    user_agent: Option<String>,
    method: Method,
) -> Result<String, AppError> {
    let value = crypto::random_token()?;
    let now: DateTime<FixedOffset> = Utc::now().into();
    ActiveModel {
        id: Set(Uuid::new_v4()),
        token_hash: Set(crypto::digest(&value)),
        user_id: Set(user_id),
        space_id: Set(space.map(SpaceId::uuid)),
        created_at: Set(now),
        last_seen_at: Set(now),
        expires_at: Set(now + ABSOLUTE),
        user_agent: Set(user_agent),
        login_method: Set(method.as_str().to_string()),
    }
    .insert(db)
    .await?;
    Ok(value)
}

pub async fn find(db: &impl ConnectionTrait, id: Uuid) -> Result<Option<Model>, AppError> {
    Ok(Entity::find_by_id(id).one(db).await?)
}

pub async fn delete(db: &impl ConnectionTrait, id: Uuid) -> Result<(), AppError> {
    Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

/// The session behind a cookie value, if any (whoever it belongs to).
pub async fn delete_by_cookie(db: &impl ConnectionTrait, cookie: &str) -> Result<(), AppError> {
    Entity::delete_many()
        .filter(Column::TokenHash.eq(crypto::digest(cookie)))
        .exec(db)
        .await?;
    Ok(())
}

/// Every session of the user except `keep` (all hosts), and every pending
/// TOTP login of the user (a password change / reset or "sign out
/// elsewhere" must also end a half-finished login).
pub async fn delete_others(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    keep: Option<Uuid>,
) -> Result<(), AppError> {
    super::entity::mfa_login::Entity::delete_many()
        .filter(super::entity::mfa_login::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    let mut q = Entity::delete_many().filter(Column::UserId.eq(user_id));
    if let Some(id) = keep {
        q = q.filter(Column::Id.ne(id));
    }
    q.exec(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_attributes() {
        let c = set_cookie("abc", true);
        assert!(c.starts_with("invoice_session=abc; HttpOnly; SameSite=Lax; Path=/;"));
        assert!(c.contains("Max-Age=7776000"));
        assert!(c.ends_with("; Secure"));
        assert!(!c.contains("Domain"));
        assert!(!set_cookie("abc", false).contains("Secure"));
        assert!(clear_cookie(false).contains("Max-Age=0"));
    }

    #[test]
    fn reads_cookie() {
        let mut h = HeaderMap::new();
        assert_eq!(cookie_value(&h), None);
        h.insert(
            header::COOKIE,
            "a=1; invoice_session=tok; b=2".parse().unwrap(),
        );
        assert_eq!(cookie_value(&h).as_deref(), Some("tok"));
        h.insert(
            header::COOKIE,
            "invoice_sessionx=1; invoice_session=".parse().unwrap(),
        );
        assert_eq!(cookie_value(&h), None);
    }

    #[test]
    fn methods() {
        for m in [Method::Password, Method::Google] {
            assert_eq!(Method::parse(m.as_str()), Some(m));
        }
        assert_eq!(Method::parse("sso"), None);
    }

    #[test]
    fn user_agent_is_capped() {
        let mut h = HeaderMap::new();
        h.insert(header::USER_AGENT, "x".repeat(300).parse().unwrap());
        assert_eq!(user_agent(&h).map(|s| s.len()), Some(200));
    }
}
