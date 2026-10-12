//! Hidden per-space limits and the storage they bound
//! (`docs/api/spaces.md`, "Limits").
//!
//! A `space_limits` row is created with every space; no route changes it,
//! an operator does with SQL. A missing row means the defaults.
//!
//! The quota counts what the database knows without listing storage: the
//! sizes of every document's original and archived PDF (summed from
//! `documents`, so a delete lowers it in the same transaction) plus the
//! design bytes recorded by the design editor (`space_storage_usage`).
//!
//! Every quota check and every record of the design bytes runs under the
//! space's quota lock ([`lock`]), and whatever grows the design bytes records
//! the new total in the transaction that holds it — so a check never sees a
//! total lower than what is stored (a stale value can only be too high,
//! until the next record after a deletion).

use chrono::Utc;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ConnectionTrait, EntityTrait, Set, Statement};
use serde::Serialize;
use utoipa::ToSchema;

use super::SpaceId;
use super::entity::{limits, usage};
use crate::error::AppError;

const MIB: u64 = 1024 * 1024;
/// Must match the column default of `space_limits.storage_quota_bytes`.
pub const DEFAULT_STORAGE_QUOTA_BYTES: u64 = 100 * MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub storage_quota_bytes: u64,
    /// Outbound e-mails on behalf of the space ([`crate::email::limits`]).
    pub email: EmailLimits,
}

/// E-mails per rolling hour / day: of the space, and of the sending user
/// within the space (their sends from other spaces do not count).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmailLimits {
    pub space_per_hour: u32,
    pub space_per_day: u32,
    pub user_per_hour: u32,
    pub user_per_day: u32,
}

impl Default for EmailLimits {
    /// Must match the column defaults (`m20261024_000001_abuse_limits`).
    fn default() -> Self {
        Self {
            space_per_hour: 20,
            space_per_day: 100,
            user_per_hour: 20,
            user_per_day: 100,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            storage_quota_bytes: DEFAULT_STORAGE_QUOTA_BYTES,
            email: EmailLimits::default(),
        }
    }
}

/// `GET /api/space` → `limits` (read-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub storage_quota_bytes: u64,
    /// Originals, archived PDFs and design files (live, draft, versions).
    pub storage_used_bytes: u64,
}

fn unsigned(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn count(n: i32) -> u32 {
    u32::try_from(n).unwrap_or(0)
}

/// The space's limits; the defaults when it has no row.
pub async fn limits(db: &impl ConnectionTrait, space: SpaceId) -> Result<Limits, AppError> {
    Ok(limits::Entity::find_by_id(space.uuid())
        .one(db)
        .await?
        .map_or_else(Limits::default, |r| Limits {
            storage_quota_bytes: unsigned(r.storage_quota_bytes),
            email: EmailLimits {
                space_per_hour: count(r.email_space_per_hour),
                space_per_day: count(r.email_space_per_day),
                user_per_hour: count(r.email_user_per_hour),
                user_per_day: count(r.email_user_per_day),
            },
        }))
}

/// The default row of a new space (inside its creating transaction).
pub async fn seed(db: &impl ConnectionTrait, space: SpaceId) -> Result<(), AppError> {
    let now = Utc::now();
    limits::Entity::insert(limits::ActiveModel {
        space_id: Set(space.uuid()),
        storage_quota_bytes: Set(
            i64::try_from(DEFAULT_STORAGE_QUOTA_BYTES).expect("100 MiB fits in i64")
        ),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        // The e-mail limits keep their column defaults.
        ..Default::default()
    })
    .exec_without_returning(db)
    .await?;
    // A new space stores no design file yet.
    record_design(db, space, 0).await
}

/// Bytes of every original and archived PDF of the space (an archive
/// written before its size was recorded counts 0).
pub async fn document_bytes(db: &impl ConnectionTrait, space: SpaceId) -> Result<u64, AppError> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COALESCE(SUM(COALESCE(original_size, 0) + COALESCE(pdf_size, 0)), 0)::bigint \
             AS n FROM documents WHERE space_id = $1",
            [space.into()],
        ))
        .await?;
    let n = match row {
        Some(r) => r.try_get::<i64>("", "n")?,
        None => 0,
    };
    Ok(unsigned(n))
}

/// Whether the design bytes were ever recorded (spaces created before the
/// limits migration start without; the design editor records them).
pub async fn has_design_usage(db: &impl ConnectionTrait, space: SpaceId) -> Result<bool, AppError> {
    Ok(usage::Entity::find_by_id(space.uuid())
        .one(db)
        .await?
        .is_some())
}

/// The design bytes as last recorded (0 before the first record).
pub async fn design_bytes(db: &impl ConnectionTrait, space: SpaceId) -> Result<u64, AppError> {
    Ok(usage::Entity::find_by_id(space.uuid())
        .one(db)
        .await?
        .map_or(0, |r| unsigned(r.design_bytes)))
}

/// Record the stored design bytes; the caller holds [`lock`] and listed
/// `design*/` (or knows the new total) inside the same transaction.
pub async fn record_design(
    db: &impl ConnectionTrait,
    space: SpaceId,
    bytes: u64,
) -> Result<(), AppError> {
    usage::Entity::insert(usage::ActiveModel {
        space_id: Set(space.uuid()),
        design_bytes: Set(i64::try_from(bytes).unwrap_or(i64::MAX)),
        updated_at: Set(Utc::now().into()),
    })
    .on_conflict(
        OnConflict::column(usage::Column::SpaceId)
            .update_columns([usage::Column::DesignBytes, usage::Column::UpdatedAt])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

pub async fn info(db: &impl ConnectionTrait, space: SpaceId) -> Result<StorageInfo, AppError> {
    Ok(StorageInfo {
        storage_quota_bytes: limits(db, space).await?.storage_quota_bytes,
        storage_used_bytes: document_bytes(db, space).await? + design_bytes(db, space).await?,
    })
}

/// A batch pre-check (ISDOC confirm): is there room for `added` bytes now?
/// Not serialized — each entry is checked again when it is stored.
pub async fn check_room(
    db: &impl ConnectionTrait,
    space: SpaceId,
    added: u64,
) -> Result<(), AppError> {
    let now = info(db, space).await?;
    check(now.storage_used_bytes, added, 0, now.storage_quota_bytes)
}

/// Serialize the quota checks of the space until the transaction ends, so
/// two concurrent uploads cannot both pass on the same remaining space.
/// Callers take it last, after their own row locks: document writes have
/// already put their object and only commit; a design upload still writes
/// to the storage while holding it. Waiting behind such a put can outlast
/// the pool's 30 s `statement_timeout`, so the rest of this transaction —
/// already short, or deliberately waiting on the storage — runs without it.
pub async fn lock(txn: &impl ConnectionTrait, space: SpaceId) -> Result<(), AppError> {
    txn.execute_unprepared("SET LOCAL statement_timeout = 0")
        .await?;
    crate::db::advisory_xact_lock(txn, format!("space-quota:{space}")).await?;
    Ok(())
}

/// A document file in the transaction: its row already counts `added` bytes
/// and no longer the `freed` ones it replaces.
pub async fn check_documents(
    txn: &impl ConnectionTrait,
    space: SpaceId,
    added: u64,
    freed: u64,
) -> Result<(), AppError> {
    if added <= freed {
        return Ok(());
    }
    lock(txn, space).await?;
    let after = document_bytes(txn, space).await? + design_bytes(txn, space).await?;
    let quota = limits(txn, space).await?.storage_quota_bytes;
    check((after + freed).saturating_sub(added), added, freed, quota)
}

/// A design upload: `design` = the bytes stored under `design*/` now (the
/// editor listed them under its design lock), the upload adds `added` and
/// replaces `freed`. Records the new total in `txn`, so it commits together
/// with the upload and the quota lock is released only then.
pub async fn check_design(
    txn: &impl ConnectionTrait,
    space: SpaceId,
    design: u64,
    added: u64,
    freed: u64,
) -> Result<(), AppError> {
    lock(txn, space).await?;
    let used = document_bytes(txn, space).await? + design;
    let quota = limits(txn, space).await?.storage_quota_bytes;
    check(used, added, freed, quota)?;
    record_design(txn, space, design.saturating_sub(freed) + added).await
}

/// Refuse a write that grows the usage past the quota. A write that does
/// not grow it always passes (a quota lowered below the usage must not
/// block replacing a file by a smaller one).
pub fn check(used: u64, added: u64, freed: u64, quota: u64) -> Result<(), AppError> {
    if added > freed && used.saturating_sub(freed) + added > quota {
        return Err(AppError::QuotaExceeded(format!(
            "{} / {} MB",
            megabytes(used),
            whole_megabytes(quota)
        )));
    }
    Ok(())
}

/// `12.3` (MiB, one decimal), the `used` part of a quota `detail`.
pub fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / MIB as f64)
}

/// `100` for a whole number of MiB, else one decimal.
fn whole_megabytes(bytes: u64) -> String {
    if bytes.is_multiple_of(MIB) {
        (bytes / MIB).to_string()
    } else {
        megabytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(r: Result<(), AppError>) -> Option<String> {
        match r {
            Err(AppError::QuotaExceeded(d)) => Some(d),
            Err(other) => panic!("unexpected {other:?}"),
            Ok(()) => None,
        }
    }

    #[test]
    fn check_counts_what_a_write_adds_and_frees() {
        let q = DEFAULT_STORAGE_QUOTA_BYTES;
        assert_eq!(detail(check(0, q, 0, q)), None, "exactly the quota fits");
        assert_eq!(
            detail(check(1, q, 0, q)).as_deref(),
            Some("0.0 / 100 MB"),
            "one byte over"
        );
        assert_eq!(
            detail(check(90 * MIB, 11 * MIB, 0, q)).as_deref(),
            Some("90.0 / 100 MB")
        );
        // Replacing a 5 MiB file by a 15 MiB one at 90 MiB used: 100 MiB.
        assert_eq!(detail(check(90 * MIB, 15 * MIB, 5 * MIB, q)), None);
        assert!(detail(check(90 * MIB, 16 * MIB, 5 * MIB, q)).is_some());
    }

    #[test]
    fn a_write_that_does_not_grow_the_usage_always_passes() {
        // The operator lowered the quota below what is stored.
        assert_eq!(detail(check(50 * MIB, 0, 0, 10 * MIB)), None);
        assert_eq!(detail(check(50 * MIB, 3 * MIB, 4 * MIB, 10 * MIB)), None);
        assert_eq!(detail(check(50 * MIB, 4 * MIB, 4 * MIB, 10 * MIB)), None);
        assert!(detail(check(50 * MIB, 5 * MIB, 4 * MIB, 10 * MIB)).is_some());
    }

    #[test]
    fn megabyte_formats() {
        assert_eq!(megabytes(0), "0.0");
        assert_eq!(megabytes(46 * MIB), "46.0");
        assert_eq!(megabytes(MIB + MIB / 2), "1.5");
        assert_eq!(megabytes(MIB + 300 * 1024), "1.3");
        assert_eq!(whole_megabytes(DEFAULT_STORAGE_QUOTA_BYTES), "100");
        assert_eq!(whole_megabytes(MIB + MIB / 2), "1.5");
        assert_eq!(
            detail(check(0, 2 * MIB, 0, MIB + MIB / 2)).as_deref(),
            Some("0.0 / 1.5 MB")
        );
    }

    #[test]
    fn defaults() {
        assert_eq!(Limits::default().storage_quota_bytes, 100 * 1024 * 1024);
        let e = Limits::default().email;
        assert_eq!((e.space_per_hour, e.space_per_day), (20, 100));
        assert_eq!((e.user_per_hour, e.user_per_day), (20, 100));
    }
}
