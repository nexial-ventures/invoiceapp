//! `PUT` / `DELETE /api/documents/{id}/original`: the uploaded original PDF.

use axum::extract::multipart::MultipartRejection;
use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::Write;
use crate::document::original::is_pdf;
use crate::document::repo::original as repo;
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiPath;

/// Largest accepted original (20 MiB).
pub const MAX_ORIGINAL: usize = 20 * 1024 * 1024;
/// Request body limit of the upload route: the file plus multipart overhead.
pub const BODY_LIMIT: usize = MAX_ORIGINAL + 64 * 1024;

fn multipart_error(e: axum::extract::multipart::MultipartError) -> AppError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::TooLarge
    } else {
        AppError::BadRequest(e.body_text())
    }
}

/// The bytes of the `file` part (other parts are ignored); more than
/// [`MAX_ORIGINAL`] → `too_large`.
async fn file_part(mut form: Multipart) -> Result<Option<Vec<u8>>, AppError> {
    while let Some(mut field) = form.next_field().await.map_err(multipart_error)? {
        if field.name() != Some("file") {
            continue;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
            if bytes.len() + chunk.len() > MAX_ORIGINAL {
                return Err(AppError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(Some(bytes));
    }
    Ok(None)
}

#[utoipa::path(
    put,
    path = "/api/documents/{id}/original",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body(content_type = "multipart/form-data", description = "One part `file`: a PDF (`%PDF-`), at most 20 MiB"),
    responses(
        (status = 204, description = "Stored (replaces any previous original)"),
        (status = 404, body = ErrorBody),
        (status = 409, description = "`invalid_state` (a native issued document)", body = ErrorBody),
        (status = 413, description = "`too_large`", body = ErrorBody),
        (status = 422, description = "`file`: `required` / `invalid`; `quota_exceeded` (+ `detail` `<used> / <limit> MB`, the space's storage quota)", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`; nothing changed", body = ErrorBody),
    )
)]
pub async fn put(
    State(state): State<AppState>,
    access: Write,
    ApiPath(id): ApiPath<Uuid>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<StatusCode, AppError> {
    let form = form.map_err(|e| AppError::BadRequest(e.body_text()))?;
    let bytes = file_part(form)
        .await?
        .ok_or(AppError::field("file", "required"))?;
    if !is_pdf(&bytes) {
        return Err(AppError::field("file", "invalid"));
    }
    repo::put(&state.db, &state.pdf.space(access.space())?, id, bytes).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/documents/{id}/original",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204),
        (status = 404, body = ErrorBody),
        (status = 409, description = "`invalid_state` (a native issued document)", body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    access: Write,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    repo::delete(&state.db, &state.pdf.space(access.space())?, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
