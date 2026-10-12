//! `GET /api/documents/isdoc`: the issued non-draft documents of the list
//! filter as one ZIP of `.isdocx` / `.isdoc` files.

use std::collections::HashSet;
use std::io::Cursor;

use anyhow::Context as _;
use axum::extract::State;
use axum::response::Response;
use futures_util::{StreamExt as _, stream};
use sea_orm::{ColumnTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::export::{Exported, attachment, export, zip_add};
use crate::app::AppState;
use crate::auth::Read;
use crate::document::entity::document::Column;
use crate::document::line::Status;
use crate::document::repo::query;
use crate::document::wire::dto::ListQuery;
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiQuery;
use crate::settings::doc_type::ISSUED;
use crate::time::today;

/// Most documents one bulk export may hold.
pub const MAX_BULK: u64 = 1000;

/// Documents exported (loaded, PDF fetched or rendered) at once.
const CONCURRENCY: usize = 8;

/// `name`, or `name` with `-2`, `-3`… before the extension when taken.
fn unique_name(taken: &mut HashSet<String>, name: &str) -> String {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let mut candidate = name.to_string();
    let mut n = 1;
    while !taken.insert(candidate.clone()) {
        n += 1;
        candidate = format!("{stem}-{n}.{ext}");
    }
    candidate
}

/// The archive writer on a blocking thread: entries arrive in order over
/// `rx`, compression stays off the async workers.
fn writer(
    mut rx: mpsc::Receiver<(String, Vec<u8>)>,
) -> tokio::task::JoinHandle<Result<Vec<u8>, AppError>> {
    tokio::task::spawn_blocking(move || {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        while let Some((name, bytes)) = rx.blocking_recv() {
            zip_add(&mut w, &name, &bytes)?;
        }
        Ok(w.finish().context("finish zip")?.into_inner())
    })
}

/// The archive, once every entry was sent: a send fails only when the
/// writer stopped on an error, which this returns.
async fn finish(
    tx: mpsc::Sender<(String, Vec<u8>)>,
    handle: tokio::task::JoinHandle<Result<Vec<u8>, AppError>>,
) -> Result<Vec<u8>, AppError> {
    drop(tx);
    handle.await.context("zip writer task")?
}

#[utoipa::path(
    get,
    path = "/api/documents/isdoc",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(ListQuery),
    responses(
        (status = 200, description = "`isdoc-export.zip` of the matching issued non-draft documents", content_type = "application/zip"),
        (status = 422, description = "`filter`: `too_many` (more than 1000)", body = ErrorBody),
        (status = 503, description = "`storage_unavailable` (the export stops at the first document it hits)", body = ErrorBody),
    )
)]
pub async fn bulk(
    State(state): State<AppState>,
    access: Read,
    ApiQuery(mut q): ApiQuery<ListQuery>,
) -> Result<Response, AppError> {
    q.direction = Some(ISSUED.into());
    let space = access.space();
    let select = || {
        query::export_select(space, &q, today()).filter(Column::Status.ne(Status::Draft.as_str()))
    };
    if select().count(&state.db).await? > MAX_BULK {
        return Err(AppError::field("filter", "too_many"));
    }
    let docs: Vec<(Uuid, Option<String>)> = select()
        .select_only()
        .column(Column::Id)
        .column(Column::Number)
        .order_by_asc(Column::IssueDate)
        .order_by_asc(Column::Number)
        .into_tuple()
        .all(&state.db)
        .await?;
    let pdf = state.pdf.space(space)?;
    // Each document on its own task: a dropped request must not cut a lazy
    // archive between its storage put and the row update. In order
    // (`buffered`, not unordered): entry order and the `-2` suffixes stay
    // those of the list.
    let mut exports = stream::iter(docs)
        .map(|(id, number)| {
            let (db, pdf) = (state.db.clone(), pdf.clone());
            let task = tokio::spawn(async move { export(&db, &pdf, id).await });
            async move {
                let result = match task.await {
                    Ok(r) => r,
                    Err(e) => Err(anyhow::Error::new(e).context("ISDOC export task").into()),
                };
                (id, number, result)
            }
        })
        .buffered(CONCURRENCY);
    let (tx, rx) = mpsc::channel(CONCURRENCY);
    let handle = writer(rx);
    // A document that fails is skipped and listed in `errors.txt` instead
    // of failing the export.
    let (mut taken, mut errors, mut first_error, mut written) =
        (HashSet::new(), String::new(), None, 0usize);
    while let Some((id, number, result)) = exports.next().await {
        match result {
            Ok(e) => {
                let name = unique_name(&mut taken, &e.filename);
                if tx.send((name, e.bytes)).await.is_err() {
                    // The writer stopped: its error is the export's.
                    finish(tx, handle).await?;
                    return Err(anyhow::anyhow!("zip writer stopped without an error").into());
                }
                written += 1;
            }
            // Every further document would fail the same way: the whole
            // export is unavailable, not a partial archive.
            Err(e @ AppError::StorageUnavailable(_)) => return Err(e),
            Err(e) => {
                tracing::warn!(document = %id, error = %e, "ISDOC bulk export skipped a document");
                let number = number.unwrap_or_else(|| id.to_string());
                errors.push_str(&format!(
                    "{number}: export failed ({})\n",
                    e.status_and_code().1
                ));
                first_error.get_or_insert(e);
            }
        }
    }
    if let (0, Some(e)) = (written, first_error) {
        return Err(e);
    }
    if !errors.is_empty() {
        // A closed channel surfaces as the writer's error in `finish`.
        let _ = tx.send(("errors.txt".into(), errors.into_bytes())).await;
    }
    Ok(attachment(Exported {
        filename: "isdoc-export.zip".into(),
        content_type: "application/zip",
        bytes: finish(tx, handle).await?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_unique() {
        let mut taken = HashSet::new();
        assert_eq!(unique_name(&mut taken, "1.isdocx"), "1.isdocx");
        assert_eq!(unique_name(&mut taken, "1.isdocx"), "1-2.isdocx");
        assert_eq!(unique_name(&mut taken, "1.isdocx"), "1-3.isdocx");
        assert_eq!(unique_name(&mut taken, "1.isdoc"), "1.isdoc");
    }
}
