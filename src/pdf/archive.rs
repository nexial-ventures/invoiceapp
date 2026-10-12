//! The archive of issued documents: rendered once, stored, never re-rendered.

use anyhow::Context as _;
use bytes::Bytes;
use futures_util::TryStreamExt as _;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement, TransactionTrait};
use uuid::Uuid;

use super::{PdfService, payload, source};
use crate::document::entity::document;
use crate::document::line::Status;
use crate::document::repo::query;
use crate::error::AppError;
use crate::settings::doc_type::RECEIVED;
use crate::storage::{self, Download};

/// `documents/{numberYear}/{id}.pdf`.
pub fn relative_path(number_year: i32, id: Uuid) -> String {
    format!("documents/{number_year}/{id}.pdf")
}

/// A PDF to send: freshly rendered bytes, or a stored object streamed from
/// the storage.
pub enum PdfBody {
    Rendered(Bytes),
    Stored(Download),
}

impl PdfBody {
    pub async fn into_bytes(self) -> Result<Bytes, AppError> {
        match self {
            Self::Rendered(bytes) => Ok(bytes),
            Self::Stored(d) => {
                let chunks: Vec<_> = d
                    .stream
                    .try_collect()
                    .await
                    .map_err(storage::Error::Unavailable)?;
                Ok(Bytes::from(chunks.concat()))
            }
        }
    }
}

/// Render the document as stored in `db` (draft → live data and watermark,
/// cancelled → "STORNO" watermark; neither gets a QR code).
pub async fn render<C: ConnectionTrait>(
    db: &C,
    pdf: &PdfService,
    id: Uuid,
) -> Result<(document::Model, Bytes), AppError> {
    let src = source::load(db, pdf.space_id(), id).await?;
    let data = payload::build(&src.input()?)?;
    let bytes = pdf.render(data).await?;
    Ok((src.row, Bytes::from(bytes)))
}

/// Record the archive on the row unless one is already recorded, then write
/// the file. Runs inside `txn`, so the file exists before anyone can see the
/// path; `None` when another writer won.
async fn store<C: ConnectionTrait>(
    txn: &C,
    pdf: &PdfService,
    row: &document::Model,
    bytes: &Bytes,
) -> Result<Option<String>, AppError> {
    let year = row
        .number_year
        .with_context(|| format!("issued document {} has no number year", row.id))?;
    let rel = relative_path(year, row.id);
    let updated = txn
        .execute(Statement::from_sql_and_values(
            txn.get_database_backend(),
            "UPDATE documents SET pdf_path = $2, pdf_sha256 = $3, pdf_size = $4, \
             pdf_rendered_at = now() WHERE id = $1 AND pdf_path IS NULL",
            [
                row.id.into(),
                rel.clone().into(),
                storage::sha256_hex(bytes).into(),
                // Counted by the space quota, never checked: issuing is not blocked.
                i64::try_from(bytes.len()).unwrap_or(i64::MAX).into(),
            ],
        ))
        .await?;
    if updated.rows_affected() == 0 {
        return Ok(None);
    }
    pdf.storage().put(&rel, bytes.clone()).await?;
    Ok(Some(rel))
}

/// Issue: render the just-issued document (row locked by the caller) and
/// archive it in the issue transaction. Returns the written path so the
/// caller can remove it if the commit fails.
pub async fn archive_in<C: ConnectionTrait>(
    txn: &C,
    pdf: &PdfService,
    id: Uuid,
) -> Result<Option<String>, AppError> {
    let (row, bytes) = render(txn, pdf, id).await?;
    store(txn, pdf, &row, &bytes).await
}

/// Render and archive an issued document that has no archive yet (a DDPP
/// after its payment, or on its first download). Concurrent callers race on
/// `WHERE pdf_path IS NULL`; the loser serves the winner's stored file, so
/// every download of a document returns the same bytes.
pub async fn archive_missing(
    db: &DatabaseConnection,
    pdf: &PdfService,
    id: Uuid,
) -> Result<Bytes, AppError> {
    let (row, bytes) = render(db, pdf, id).await?;
    let txn = db.begin().await?;
    let stored = match store(&txn, pdf, &row, &bytes).await {
        Ok(s) => s,
        Err(e) => {
            if let Err(r) = txn.rollback().await {
                tracing::warn!(error = %r, "rollback PDF archive");
            }
            return Err(e);
        }
    };
    if let Err(e) = txn.commit().await {
        if let Some(rel) = &stored {
            pdf.storage().remove(rel).await;
        }
        return Err(e.into());
    }
    if stored.is_some() {
        return Ok(bytes);
    }
    // The losing UPDATE waited for the winner's commit, so its path is set.
    let row = query::find(db, pdf.space_id(), id).await?;
    let rel = row
        .pdf_path
        .with_context(|| format!("document {id} lost the archive race but has no archive"))?;
    Ok(pdf.storage().get(&rel).await?)
}

/// After an auto-issued DDPP's payment committed: archive it now if mdcast
/// is up; otherwise its first download does.
pub async fn try_archive_ddpp(db: &DatabaseConnection, pdf: &PdfService, id: Uuid) {
    if let Err(e) = archive_missing(db, pdf, id).await {
        tracing::warn!(document = %id, error = %e, "DDPP PDF not archived yet");
    }
}

/// [`try_archive_ddpp`] in the background, so mdcast never delays the
/// payment response.
pub fn spawn_archive_ddpp(db: DatabaseConnection, pdf: PdfService, id: Uuid) {
    tokio::spawn(async move { try_archive_ddpp(&db, &pdf, id).await });
}

/// The PDF for `GET /api/documents/{id}/pdf`: a received or imported
/// document serves its uploaded original (none → `pdf_missing`), never a
/// render; a draft is rendered live and never stored; an issued or cancelled
/// document serves its archive (stored ones streamed).
/// Received and imported documents are never rendered: their PDF is the
/// uploaded original, if any.
pub fn serves_original(direction: &str, imported: bool) -> bool {
    direction == RECEIVED || imported
}

pub async fn document_pdf(
    db: &DatabaseConnection,
    pdf: &PdfService,
    id: Uuid,
) -> Result<(document::Model, PdfBody), AppError> {
    let row = query::find(db, pdf.space_id(), id).await?;
    if serves_original(&row.direction, row.imported) {
        let rel = row.original_path.as_deref().ok_or(AppError::PdfMissing)?;
        let body = PdfBody::Stored(pdf.storage().get_stream(rel).await?);
        return Ok((row, body));
    }
    if row.status == Status::Draft.as_str() {
        let (row, bytes) = render(db, pdf, id).await?;
        return Ok((row, PdfBody::Rendered(bytes)));
    }
    let body = match &row.pdf_path {
        Some(rel) => PdfBody::Stored(pdf.storage().get_stream(rel).await?),
        None => PdfBody::Rendered(archive_missing(db, pdf, id).await?),
    };
    Ok((row, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_path() {
        assert_eq!(
            relative_path(2026, Uuid::nil()),
            "documents/2026/00000000-0000-0000-0000-000000000000.pdf"
        );
    }
}
