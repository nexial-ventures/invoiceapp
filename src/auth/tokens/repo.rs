//! Personal API tokens: one user in one space, sha256 of the whole token
//! stored with its prefix; `last_used_at` written at most once a minute.

use chrono::{NaiveDate, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use uuid::Uuid;

use crate::auth::crypto;
use crate::auth::entity::{api_token, user};
use crate::error::AppError;
use crate::space::{Role, SpaceId};

/// Valid through the end of its `expires_at` day (UTC).
pub fn expired(expires_at: Option<NaiveDate>, today: NaiveDate) -> bool {
    expires_at.is_some_and(|d| d < today)
}

/// Tokens of `space`, oldest first; `owner` limits them to one user's.
pub async fn list(
    db: &DatabaseConnection,
    space: SpaceId,
    owner: Option<Uuid>,
) -> Result<Vec<(api_token::Model, Option<user::Model>)>, AppError> {
    let mut q = api_token::Entity::find().filter(api_token::Column::SpaceId.eq(space));
    if let Some(user_id) = owner {
        q = q.filter(api_token::Column::UserId.eq(user_id));
    }
    let rows = q
        .order_by_asc(api_token::Column::CreatedAt)
        .order_by_asc(api_token::Column::Id)
        .all(db)
        .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.user_id).collect();
    let users = user::Entity::find()
        .filter(user::Column::Id.is_in(ids))
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let u = users.iter().find(|u| u.id == r.user_id).cloned();
            (r, u)
        })
        .collect())
}

#[derive(Debug)]
pub struct NewToken {
    pub name: String,
    pub role: Role,
    pub expires_at: Option<NaiveDate>,
}

/// Store a new token; returns the row and the token (shown once).
pub async fn create(
    db: &DatabaseConnection,
    space: SpaceId,
    user_id: Uuid,
    new: NewToken,
) -> Result<(api_token::Model, String), AppError> {
    let (token, prefix) = crypto::new_api_token()?;
    let row = api_token::ActiveModel {
        id: Set(Uuid::new_v4()),
        space_id: Set(space.uuid()),
        user_id: Set(user_id),
        name: Set(new.name),
        prefix: Set(prefix),
        token_hash: Set(crypto::digest(&token)),
        role: Set(new.role.as_str().to_string()),
        created_at: Set(Utc::now().into()),
        expires_at: Set(new.expires_at),
        last_used_at: Set(None),
    }
    .insert(db)
    .await?;
    Ok((row, token))
}

/// The expiry of token `id` (`None`: no expiry); 401 when it was revoked
/// meanwhile.
pub async fn expiry(db: &DatabaseConnection, id: Uuid) -> Result<Option<NaiveDate>, AppError> {
    api_token::Entity::find_by_id(id)
        .one(db)
        .await?
        .map(|t| t.expires_at)
        .ok_or(AppError::Unauthorized)
}

/// Delete token `id` of `space` (only `owner`'s when given); `false` when
/// there is no such token.
pub async fn delete(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    owner: Option<Uuid>,
) -> Result<bool, AppError> {
    let mut q = api_token::Entity::delete_many()
        .filter(api_token::Column::Id.eq(id))
        .filter(api_token::Column::SpaceId.eq(space));
    if let Some(user_id) = owner {
        q = q.filter(api_token::Column::UserId.eq(user_id));
    }
    Ok(q.exec(db).await?.rows_affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_is_end_of_day() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
        let today = d("2026-10-10").expect("date");
        assert!(!expired(None, today));
        assert!(!expired(d("2026-10-10"), today));
        assert!(!expired(d("2026-10-11"), today));
        assert!(expired(d("2026-10-09"), today));
    }
}
