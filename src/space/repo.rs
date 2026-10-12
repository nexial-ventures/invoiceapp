//! Spaces and memberships.

use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, Statement, TransactionTrait,
};
use uuid::Uuid;

use super::entity::{member, space};
use super::{Role, SpaceId, seed};
use crate::error::{AppError, unique_violation};

pub async fn find_by_slug(
    db: &DatabaseConnection,
    slug: &str,
) -> Result<Option<space::Model>, AppError> {
    Ok(space::Entity::find()
        .filter(space::Column::Slug.eq(slug))
        .one(db)
        .await?)
}

pub async fn find(
    db: &impl ConnectionTrait,
    id: SpaceId,
) -> Result<Option<space::Model>, AppError> {
    Ok(space::Entity::find_by_id(id.uuid()).one(db).await?)
}

/// The user's role in `space`, `None` without a membership.
pub async fn member_role(
    db: &impl ConnectionTrait,
    space: SpaceId,
    user_id: Uuid,
) -> Result<Option<Role>, AppError> {
    Ok(member::Entity::find_by_id((space.uuid(), user_id))
        .one(db)
        .await?
        .and_then(|m| Role::parse(&m.role)))
}

/// The user's spaces with their role, by name.
pub async fn list_for_user(
    db: &DatabaseConnection,
    user_id: Uuid,
) -> Result<Vec<(space::Model, Role)>, AppError> {
    let members = member::Entity::find()
        .filter(member::Column::UserId.eq(user_id))
        .all(db)
        .await?;
    let ids: Vec<Uuid> = members.iter().map(|m| m.space_id).collect();
    let spaces = space::Entity::find()
        .filter(space::Column::Id.is_in(ids))
        .all(db)
        .await?;
    let mut out: Vec<(space::Model, Role)> = spaces
        .into_iter()
        .filter_map(|s| {
            let m = members.iter().find(|m| m.space_id == s.id)?;
            Some((s, Role::parse(&m.role)?))
        })
        .collect();
    out.sort_by(|(a, _), (b, _)| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.slug.cmp(&b.slug))
    });
    Ok(out)
}

/// Create a space owned by `owner` with every per-space seed, in one
/// transaction. A taken slug → 422 `slug: taken`.
pub async fn create(
    db: &DatabaseConnection,
    slug: String,
    name: String,
    owner: Uuid,
) -> Result<space::Model, AppError> {
    let txn = db.begin().await?;
    let now = Utc::now();
    let id = Uuid::new_v4();
    let inserted = space::ActiveModel {
        id: Set(id),
        slug: Set(slug),
        name: Set(name),
        created_at: Set(now.into()),
        require_mfa: Set(false),
    }
    .insert(&txn)
    .await;
    let row = match inserted {
        Ok(row) => row,
        Err(e) if unique_violation(&e).as_deref() == Some("spaces_slug_key") => {
            return Err(AppError::field("slug", "taken"));
        }
        Err(e) => return Err(e.into()),
    };
    member::ActiveModel {
        space_id: Set(id),
        user_id: Set(owner),
        role: Set(Role::Owner.as_str().to_string()),
        created_at: Set(now.into()),
    }
    .insert(&txn)
    .await?;
    seed::seed(&txn, SpaceId::from_row(id)).await?;
    txn.commit().await?;
    Ok(row)
}

/// Set the name and / or the TOTP policy (`None` = unchanged).
pub async fn update(
    db: &DatabaseConnection,
    space: SpaceId,
    name: Option<String>,
    require_mfa: Option<bool>,
) -> Result<space::Model, AppError> {
    if name.is_some() || require_mfa.is_some() {
        let mut q = space::Entity::update_many().filter(space::Column::Id.eq(space));
        if let Some(name) = name {
            q = q.col_expr(space::Column::Name, Expr::value(name));
        }
        if let Some(on) = require_mfa {
            q = q.col_expr(space::Column::RequireMfa, Expr::value(on));
        }
        q.exec(db).await?;
    }
    find(db, space).await?.ok_or(AppError::NotFound)
}

/// Delete the space and, by cascade, every row it owns (documents and
/// their children, contacts, settings, memberships, tokens, its sessions).
pub async fn delete(db: &DatabaseConnection, space: SpaceId) -> Result<(), AppError> {
    let txn = db.begin().await?;
    // `document_lines.advance_document_id` has no ON DELETE action: within
    // the one cascading delete its check can fire for a DDPP before the
    // deducting invoice's lines are gone (it depends on the row order of
    // the plan). Removing the space's lines first makes it order-proof.
    txn.execute(Statement::from_sql_and_values(
        txn.get_database_backend(),
        "DELETE FROM document_lines WHERE document_id IN \
         (SELECT id FROM documents WHERE space_id = $1)",
        [space.uuid().into()],
    ))
    .await?;
    space::Entity::delete_by_id(space.uuid()).exec(&txn).await?;
    txn.commit().await?;
    Ok(())
}
