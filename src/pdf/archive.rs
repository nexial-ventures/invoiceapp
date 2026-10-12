//! The archive of issued documents: rendered once, stored, never re-rendered.

use anyhow::Context as _;
use bytes::Bytes;
use futures_util::TryStreamExt as _;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use uuid::Uuid;

use super::{PdfService, payload, source};
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::document::line::Status;
use crate::document::repo::{original, query};
use crate::error::AppError;
use crate::storage::{self, Download};

/// `documents/{numberYear}/{id}-{sha8}-{nonce}.pdf`: a new key per write
/// attempt, so the object can be written before the row points at it, a lost
/// attempt never overwrites a committed archive, and a failed attempt's
/// cleanup only ever removes its own object (a double-clicked issue renders
/// identical bytes). Archives from before have `{id}.pdf`; the row's
/// `pdf_path` is what counts.
pub fn relative_path(number_year: i32, id: Uuid, sha256: &str, nonce: &str) -> String {
    let short = sha256.get(..8).unwrap_or(sha256);
    format!("documents/{number_year}/{id}-{short}-{nonce}.pdf")
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

/// Write the rendered archive of `id` (no transaction is open: the caller
/// records it afterwards, and removes it with
/// [`original::remove_unreferenced`] if that fails).
pub async fn put(
    pdf: &PdfService,
    number_year: i32,
    id: Uuid,
    bytes: &Bytes,
) -> Result<String, AppError> {
    let rel = relative_path(
        number_year,
        id,
        &storage::sha256_hex(bytes),
        &storage::nonce(),
    );
    pdf.storage().put(&rel, bytes.clone()).await?;
    Ok(rel)
}

/// Point the row at the written archive unless one is already recorded;
/// `false` when another writer won.
pub async fn record<C: ConnectionTrait>(
    db: &C,
    id: Uuid,
    rel: &str,
    bytes: &Bytes,
) -> Result<bool, AppError> {
    let updated = db
        .execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE documents SET pdf_path = $2, pdf_sha256 = $3, pdf_size = $4, \
             pdf_rendered_at = now() WHERE id = $1 AND pdf_path IS NULL",
            [
                id.into(),
                rel.to_string().into(),
                storage::sha256_hex(bytes).into(),
                // Counted by the space quota, never checked: issuing is not blocked.
                i64::try_from(bytes.len()).unwrap_or(i64::MAX).into(),
            ],
        ))
        .await?;
    Ok(updated.rows_affected() == 1)
}

/// Render and archive an issued document that has no archive yet (a DDPP
/// after its payment, or on its first download). Render and object write
/// run without a transaction; concurrent callers then race on `WHERE
/// pdf_path IS NULL`, and the loser drops its object and serves the
/// winner's, so every download of a document returns the same bytes.
pub async fn archive_missing(
    db: &DatabaseConnection,
    pdf: &PdfService,
    id: Uuid,
) -> Result<Bytes, AppError> {
    let (row, bytes) = render(db, pdf, id).await?;
    let year = row
        .number_year
        .with_context(|| format!("issued document {id} has no number year"))?;
    let rel = put(pdf, year, id, &bytes).await?;
    match record(db, id, &rel, &bytes).await {
        Ok(true) => return Ok(bytes),
        Ok(false) => original::remove_unreferenced(db, pdf, &rel).await,
        Err(e) => {
            original::remove_unreferenced(db, pdf, &rel).await;
            return Err(e);
        }
    }
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

/// Received and imported documents are never rendered: their PDF is the
/// uploaded original, if any.
pub fn serves_original(direction: Direction, imported: bool) -> bool {
    direction == Direction::Received || imported
}

/// The PDF for `GET /api/documents/{id}/pdf`: a received or imported
/// document serves its uploaded original (none → `pdf_missing`), never a
/// render; a draft is rendered live and never stored; an issued or cancelled
/// document serves its archive (stored ones streamed).
pub async fn document_pdf(
    db: &DatabaseConnection,
    pdf: &PdfService,
    id: Uuid,
) -> Result<(document::Model, PdfBody), AppError> {
    let row = query::find(db, pdf.space_id(), id).await?;
    if serves_original(row.direction, row.imported) {
        let rel = row.original_path.as_deref().ok_or(AppError::PdfMissing)?;
        let body = PdfBody::Stored(pdf.storage().get_stream(rel).await?);
        return Ok((row, body));
    }
    if row.status == Status::Draft {
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
            relative_path(2026, Uuid::nil(), "ba7816bf8f01cfea", "0a1b2c3d"),
            "documents/2026/00000000-0000-0000-0000-000000000000-ba7816bf-0a1b2c3d.pdf"
        );
    }
}
