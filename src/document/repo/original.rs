//! The uploaded original PDF of a received or imported document.

use bytes::Bytes;
use chrono::Datelike;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter, Set, TransactionTrait,
};
use uuid::Uuid;

use super::query;
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::error::AppError;
use crate::pdf::PdfService;
use crate::pdf::editor::ensure_usage;
use crate::space::{SpaceId, limits};
use crate::storage;

/// `documents/{year}/{id}-original-{sha8}-{nonce}.pdf`: a new key per write
/// attempt, so a replacement never overwrites the committed file and a failed
/// attempt's cleanup never removes another attempt's object.
pub fn relative_path(year: i32, id: Uuid, sha256: &str, nonce: &str) -> String {
    let short = sha256.get(..8).unwrap_or(sha256);
    format!("documents/{year}/{id}-original-{short}-{nonce}.pdf")
}

/// Received documents and imported issued ones (any status); a native
/// issued document's PDF is its rendered archive.
fn allowed(doc: &document::Model) -> Result<(), AppError> {
    if doc.direction == Direction::Received || doc.imported {
        Ok(())
    } else {
        Err(AppError::InvalidState)
    }
}

/// Best effort: remove an object a failed write, rollback or commit may have
/// left — but only when no document of the space points at it. New keys
/// carry a per-attempt nonce, so this only ever finds the caller's own
/// object; the reference check guards keys written before nonces existed.
pub async fn remove_unreferenced<C: ConnectionTrait>(db: &C, pdf: &PdfService, key: &str) {
    let storage = pdf.storage();
    let referenced = document::Entity::find()
        .filter(document::Column::SpaceId.eq(pdf.space_id()))
        .filter(
            Condition::any()
                .add(document::Column::OriginalPath.eq(key))
                .add(document::Column::PdfPath.eq(key)),
        )
        .count(db)
        .await;
    match referenced {
        Ok(0) => storage.remove(key).await,
        Ok(_) => {}
        Err(e) => tracing::warn!(key, error = %e, "keep a possibly orphaned object"),
    }
}

/// A stored original: its per-upload key, digest and size.
pub struct Original {
    pub rel: String,
    pub sha256: String,
    pub size: i64,
}

impl Original {
    pub fn new(year: i32, id: Uuid, bytes: &[u8]) -> Result<Self, AppError> {
        let sha256 = storage::sha256_hex(bytes);
        Ok(Self {
            rel: relative_path(year, id, &sha256, &storage::nonce()),
            sha256,
            size: i64::try_from(bytes.len()).map_err(|_| AppError::TooLarge)?,
        })
    }

    /// Point `row` at this original.
    pub fn apply(&self, row: &mut document::ActiveModel, now: chrono::DateTime<chrono::Utc>) {
        row.original_path = Set(Some(self.rel.clone()));
        row.original_sha256 = Set(Some(self.sha256.clone()));
        row.original_size = Set(Some(self.size));
        row.original_uploaded_at = Set(Some(now.into()));
    }
}

/// Store `bytes` (already checked to be a PDF) as the document's original,
/// replacing any previous one. The object is written **before** the
/// transaction (under a new per-upload key, so it never overwrites a
/// committed file): no row lock is held across the storage call. The old
/// file is removed only after the commit, the new one (when unreferenced) if
/// the transaction fails — also on 422 `quota_exceeded` (checked in it).
pub async fn put(
    db: &DatabaseConnection,
    pdf: &PdfService,
    id: Uuid,
    bytes: Vec<u8>,
) -> Result<(), AppError> {
    ensure_usage(db, pdf.space_id(), pdf.storage()).await?;
    let doc = query::find(db, pdf.space_id(), id).await?;
    allowed(&doc)?;
    let year = doc.number_year.unwrap_or(doc.issue_date.year());
    let original = Original::new(year, id, &bytes)?;
    pdf.storage().put(&original.rel, Bytes::from(bytes)).await?;
    let txn = match db.begin().await {
        Ok(txn) => txn,
        Err(e) => {
            // No row can point at the fresh key yet: it is ours to drop.
            pdf.storage().remove(&original.rel).await;
            return Err(e.into());
        }
    };
    let previous = match put_in(&txn, pdf.space_id(), id, &original).await {
        Ok(previous) => previous,
        Err(e) => {
            let e = rollback(txn, e).await;
            remove_unreferenced(db, pdf, &original.rel).await;
            return Err(e);
        }
    };
    if let Err(e) = txn.commit().await {
        remove_unreferenced(db, pdf, &original.rel).await;
        return Err(e.into());
    }
    if let Some(old) = previous.filter(|old| *old != original.rel) {
        pdf.storage().remove(&old).await;
    }
    Ok(())
}

async fn rollback(txn: DatabaseTransaction, e: AppError) -> AppError {
    if let Err(r) = txn.rollback().await {
        tracing::warn!(error = %r, "rollback original upload");
    }
    e
}

/// Updates the row; returns the previously stored key.
async fn put_in(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
    original: &Original,
) -> Result<Option<String>, AppError> {
    let doc = query::lock(txn, space, id).await?;
    allowed(&doc)?;
    let previous = doc.original_path.clone();
    let freed = doc
        .original_size
        .map_or(0, |n| u64::try_from(n).unwrap_or(0));
    let now = chrono::Utc::now();
    let mut row: document::ActiveModel = doc.into();
    original.apply(&mut row, now);
    row.updated_at = Set(now.into());
    row.update(txn).await?;
    // Last, under the space's quota lock; a refusal rolls back and the
    // caller removes the object it put.
    let added = u64::try_from(original.size).unwrap_or(0);
    limits::check_documents(txn, space, added, freed).await?;
    Ok(previous)
}

/// Idempotent; the file is removed after the commit (best effort).
pub async fn delete(db: &DatabaseConnection, pdf: &PdfService, id: Uuid) -> Result<(), AppError> {
    let space = pdf.space_id();
    let removed = db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = query::lock(txn, space, id).await?;
                allowed(&doc)?;
                let rel = doc.original_path.clone();
                let mut row: document::ActiveModel = doc.into();
                row.original_path = Set(None);
                row.original_sha256 = Set(None);
                row.original_size = Set(None);
                row.original_uploaded_at = Set(None);
                row.updated_at = Set(chrono::Utc::now().into());
                row.update(txn).await?;
                Ok::<_, AppError>(rel)
            })
        })
        .await?;
    if let Some(rel) = removed {
        pdf.storage().remove(&rel).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_path() {
        assert_eq!(
            relative_path(2026, Uuid::nil(), "ba7816bf8f01cfea", "0a1b2c3d"),
            "documents/2026/00000000-0000-0000-0000-000000000000-original-ba7816bf-0a1b2c3d.pdf"
        );
    }
}
