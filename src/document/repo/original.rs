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
use crate::error::AppError;
use crate::pdf::PdfService;
use crate::settings::doc_type::RECEIVED;
use crate::space::{SpaceId, limits};
use crate::storage;

/// `documents/{year}/{id}-original-{sha8}.pdf`: content-addressed, so a
/// replacement never overwrites the committed file.
pub fn relative_path(year: i32, id: Uuid, sha256: &str) -> String {
    let short = sha256.get(..8).unwrap_or(sha256);
    format!("documents/{year}/{id}-original-{short}.pdf")
}

/// Received documents and imported issued ones (any status); a native
/// issued document's PDF is its rendered archive.
fn allowed(doc: &document::Model) -> Result<(), AppError> {
    if doc.direction == RECEIVED || doc.imported {
        Ok(())
    } else {
        Err(AppError::InvalidState)
    }
}

/// Best effort: remove an object a failed write or rollback may have left —
/// but only when no document points at it. Keys are content-addressed, so a
/// re-upload of identical content shares its key with the stored row.
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

/// Store `bytes` (already checked to be a PDF) as the document's original,
/// replacing any previous one. The row is updated and the new file written
/// inside the transaction; the old file is removed only after the commit,
/// the new one (when unreferenced) if the write or the commit fails.
pub async fn put(
    db: &DatabaseConnection,
    pdf: &PdfService,
    id: Uuid,
    bytes: Vec<u8>,
) -> Result<(), AppError> {
    let txn = db.begin().await?;
    let (rel, previous) = match put_in(&txn, pdf.space_id(), id, &bytes).await {
        Ok(paths) => paths,
        Err(e) => return Err(rollback(txn, e).await),
    };
    if let Err(e) = pdf.storage().put(&rel, Bytes::from(bytes)).await {
        let e = rollback(txn, e.into()).await;
        remove_unreferenced(db, pdf, &rel).await;
        return Err(e);
    }
    if let Err(e) = txn.commit().await {
        remove_unreferenced(db, pdf, &rel).await;
        return Err(e.into());
    }
    if let Some(old) = previous.filter(|old| *old != rel) {
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

/// Updates the row; returns the new key and the previously stored one.
async fn put_in(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
    bytes: &[u8],
) -> Result<(String, Option<String>), AppError> {
    let doc = query::lock(txn, space, id).await?;
    allowed(&doc)?;
    let sha256 = storage::sha256_hex(bytes);
    let year = doc.number_year.unwrap_or(doc.issue_date.year());
    let rel = relative_path(year, id, &sha256);
    let previous = doc.original_path.clone();
    let freed = doc
        .original_size
        .map_or(0, |n| u64::try_from(n).unwrap_or(0));
    let size = i64::try_from(bytes.len()).map_err(|_| AppError::TooLarge)?;
    let now = chrono::Utc::now();
    let mut row: document::ActiveModel = doc.into();
    row.original_path = Set(Some(rel.clone()));
    row.original_sha256 = Set(Some(sha256));
    row.original_size = Set(Some(size));
    row.original_uploaded_at = Set(Some(now.into()));
    row.updated_at = Set(now.into());
    row.update(txn).await?;
    limits::check_documents(txn, space, bytes.len() as u64, freed).await?;
    Ok((rel, previous))
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
            relative_path(2026, Uuid::nil(), "ba7816bf8f01cfea"),
            "documents/2026/00000000-0000-0000-0000-000000000000-original-ba7816bf.pdf"
        );
    }
}
