//! The draft row (lock + deletions) and the version history.

use anyhow::Context as _;
use chrono::Utc;
use sea_orm::sea_query::{LockBehavior, LockType, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use uuid::Uuid;

use super::entity::{draft, version};
use super::{KEEP_VERSIONS, UserRef, Version, VersionFile};
use crate::auth::entity::user;
use crate::error::{AppError, lock_not_available};
use crate::space::SpaceId;

/// Create the space's empty draft row unless it exists (its own statement:
/// an insert inside the locking transaction could wait on a held lock).
pub async fn ensure(db: &impl ConnectionTrait, space: SpaceId) -> Result<(), AppError> {
    draft::Entity::insert(draft::ActiveModel {
        space_id: Set(space.uuid()),
        deleted: Set(Vec::new()),
        revision: Set(0),
        updated_at: Set(Utc::now().into()),
        updated_by: Set(None),
    })
    .on_conflict(
        OnConflict::column(draft::Column::SpaceId)
            .do_nothing()
            .to_owned(),
    )
    .do_nothing()
    .exec(db)
    .await?;
    Ok(())
}

/// Lock the draft row (until the transaction ends) without waiting: held
/// by another change → `design_busy`.
pub async fn lock(txn: &impl ConnectionTrait, space: SpaceId) -> Result<draft::Model, AppError> {
    let row = draft::Entity::find_by_id(space.uuid())
        .lock_with_behavior(LockType::Update, LockBehavior::Nowait)
        .one(txn)
        .await
        .map_err(|e| {
            if lock_not_available(&e) {
                AppError::DesignBusy
            } else {
                e.into()
            }
        })?
        .context("design draft row missing (created before the lock)")?;
    Ok(row)
}

/// The draft row without locking (listing, preview).
pub async fn draft(
    db: &impl ConnectionTrait,
    space: SpaceId,
) -> Result<Option<draft::Model>, AppError> {
    Ok(draft::Entity::find_by_id(space.uuid()).one(db).await?)
}

/// Record a change of the locked draft `row` by `user` with the new
/// deletions (next revision).
pub async fn touch(
    txn: &impl ConnectionTrait,
    row: &draft::Model,
    user: Uuid,
    deleted: Vec<String>,
) -> Result<(), AppError> {
    draft::ActiveModel {
        space_id: Set(row.space_id),
        deleted: Set(deleted),
        revision: Set(row.revision + 1),
        updated_at: Set(Utc::now().into()),
        updated_by: Set(Some(user)),
    }
    .update(txn)
    .await?;
    Ok(())
}

pub async fn user_ref(
    db: &impl ConnectionTrait,
    id: Option<Uuid>,
) -> Result<Option<UserRef>, AppError> {
    let Some(id) = id else { return Ok(None) };
    Ok(user::Entity::find_by_id(id)
        .one(db)
        .await?
        .map(|u| UserRef {
            email: u.email,
            display_name: u.display_name,
        }))
}

pub async fn version(
    db: &impl ConnectionTrait,
    space: SpaceId,
    id: Uuid,
) -> Result<Option<version::Model>, AppError> {
    Ok(version::Entity::find_by_id(id)
        .filter(version::Column::SpaceId.eq(space.uuid()))
        .one(db)
        .await?)
}

/// Append a version (number = the space's highest + 1; callers hold the
/// lock, so numbers cannot race).
pub async fn insert_version(
    txn: &impl ConnectionTrait,
    space: SpaceId,
    id: Uuid,
    kind: &str,
    user: Uuid,
    files: &[VersionFile],
) -> Result<(), AppError> {
    let last = version::Entity::find()
        .filter(version::Column::SpaceId.eq(space.uuid()))
        .order_by_desc(version::Column::Number)
        .one(txn)
        .await?
        .map_or(0, |v| v.number);
    version::ActiveModel {
        id: Set(id),
        space_id: Set(space.uuid()),
        number: Set(last + 1),
        kind: Set(kind.to_string()),
        created_at: Set(Utc::now().into()),
        created_by: Set(Some(user)),
        files: Set(serde_json::to_value(files).context("serialize version files")?),
    }
    .insert(txn)
    .await?;
    Ok(())
}

/// Ids of the space's versions, newest first.
pub async fn version_ids(db: &impl ConnectionTrait, space: SpaceId) -> Result<Vec<Uuid>, AppError> {
    Ok(version::Entity::find()
        .select_only()
        .column(version::Column::Id)
        .filter(version::Column::SpaceId.eq(space.uuid()))
        .order_by_desc(version::Column::Number)
        .into_tuple()
        .all(db)
        .await?)
}

/// Delete the rows beyond the [`KEEP_VERSIONS`] newest; returns the ids
/// deleted (their objects go after the commit).
pub async fn prune(txn: &impl ConnectionTrait, space: SpaceId) -> Result<Vec<Uuid>, AppError> {
    let ids = version_ids(txn, space).await?;
    let keep = usize::try_from(KEEP_VERSIONS).unwrap_or(usize::MAX);
    let old = ids.get(keep..).unwrap_or_default().to_vec();
    if !old.is_empty() {
        version::Entity::delete_many()
            .filter(version::Column::Id.is_in(old.clone()))
            .exec(txn)
            .await?;
    }
    Ok(old)
}

/// The space's versions, newest first.
pub async fn versions(db: &impl ConnectionTrait, space: SpaceId) -> Result<Vec<Version>, AppError> {
    let rows = version::Entity::find()
        .filter(version::Column::SpaceId.eq(space.uuid()))
        .order_by_desc(version::Column::Number)
        .all(db)
        .await?;
    let user_ids: Vec<Uuid> = rows.iter().filter_map(|r| r.created_by).collect();
    let users = user::Entity::find()
        .filter(user::Column::Id.is_in(user_ids))
        .all(db)
        .await?;
    rows.into_iter()
        .enumerate()
        .map(|(i, r)| {
            let created_by = users
                .iter()
                .find(|u| Some(u.id) == r.created_by)
                .map(|u| UserRef {
                    email: u.email.clone(),
                    display_name: u.display_name.clone(),
                });
            let files: Vec<VersionFile> =
                serde_json::from_value(r.files).context("design_versions.files")?;
            Ok(Version {
                id: r.id,
                number: r.number,
                kind: r.kind,
                created_at: r.created_at,
                created_by,
                files,
                current: i == 0,
            })
        })
        .collect()
}
