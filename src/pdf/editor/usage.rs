//! The space's design bytes for its storage quota (`space::limits`): live +
//! draft + kept versions, listed and recorded under the space's quota lock,
//! so a concurrent quota check waits for the record instead of reading a
//! total lower than what is stored.

use sea_orm::{DatabaseConnection, DatabaseTransaction, EntityTrait};

use super::files::Stored;
use super::{begin, in_txn};
use crate::error::AppError;
use crate::space::entity::space;
use crate::space::{SpaceId, limits};
use crate::storage::Storage;

/// Inside `txn` (a design change, after its storage writes): take the quota
/// lock, list `design*/` and record the total; it commits with `txn`.
pub(super) async fn record_in(
    txn: &DatabaseTransaction,
    space: SpaceId,
    storage: &Storage,
) -> Result<Stored, AppError> {
    limits::lock(txn, space).await?;
    let stored = Stored::load(storage).await?;
    limits::record_design(txn, space, stored.usage()).await?;
    Ok(stored)
}

/// [`record_in`] in a transaction of its own; returns the listing.
pub(super) async fn recorded(
    db: &DatabaseConnection,
    space: SpaceId,
    storage: &Storage,
) -> Result<Stored, AppError> {
    let txn = begin(db).await?;
    let result = record_in(&txn, space, storage).await;
    in_txn(txn, result).await
}

/// After a change that only removed files (the recorded total was too high
/// meanwhile, never too low); best effort — the next listing records again.
pub async fn refresh_usage(db: &DatabaseConnection, space: SpaceId, storage: &Storage) {
    if let Err(e) = recorded(db, space, storage).await {
        tracing::warn!(space = %space, error = %e, "record the design storage usage");
    }
}

/// Record the design bytes once for a space that has none recorded (created
/// before the limits migration) — before its first quota check or usage.
pub async fn ensure_usage(
    db: &DatabaseConnection,
    space: SpaceId,
    storage: &Storage,
) -> Result<(), AppError> {
    if !limits::has_design_usage(db, space).await? {
        recorded(db, space, storage).await?;
    }
    Ok(())
}

/// Every space, after `invoice storage migrate` copied keys that may hold
/// their files (`spaces/{id}/design…`); returns how many were recorded.
pub async fn refresh_all(db: &DatabaseConnection, root: &Storage) -> Result<usize, AppError> {
    let spaces = space::Entity::find().all(db).await?;
    for s in &spaces {
        let id = SpaceId::from_row(s.id);
        refresh_usage(db, id, &root.scoped(&id.storage_prefix())?).await;
    }
    Ok(spaces.len())
}
