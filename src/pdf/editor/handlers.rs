//! The design listing, file downloads and the draft routes.

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::files::Stored;
use super::{
    DRAFT_PREFIX, QUOTA_BYTES, UserRef, draft_key, in_txn, locked, refresh_usage, repo, safe_path,
    usage, valid_upload_path,
};
use crate::app::AppState;
use crate::auth::{Manage, Read};
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiQuery;
use crate::pdf::PdfService;
use crate::pdf::design::{self, DesignFile, MAX_FILE_BYTES, TEMPLATE};
use crate::pdf::handlers::safe_filename;
use crate::space::limits::{self, megabytes};
use crate::storage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    /// A new path.
    Added,
    /// Overrides a current file (custom or default).
    Replaced,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DraftFile {
    pub path: String,
    pub size: u64,
    pub change: Change,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftListing {
    pub files: Vec<DraftFile>,
    /// Live custom files the activation removes.
    pub deleted: Vec<String>,
    pub updated_at: Option<DateTime<FixedOffset>>,
    pub updated_by: Option<UserRef>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Quota {
    /// Bytes stored for live + draft + kept versions.
    pub used: u64,
    pub limit: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DesignListing {
    /// The storage backend the custom files come from: `fs` | `s3`.
    pub storage: String,
    pub files: Vec<DesignFile>,
    /// `null` when nothing is pending.
    pub draft: Option<DraftListing>,
    pub quota: Quota,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PathQuery {
    /// Relative design path, e.g. `logo.svg`, `fonts/brand.ttf`.
    pub path: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FileQuery {
    pub path: String,
    /// `live` (default; the effective file) | `draft` | `default`.
    pub source: Option<String>,
}

async fn listing(state: &AppState, pdf: &PdfService) -> Result<DesignListing, AppError> {
    let storage = pdf.storage();
    // The space quota counts these bytes without listing storage.
    let stored = usage::recorded(&state.db, pdf.space_id(), storage).await?;
    let files = design::effective(&stored.live_files());
    let row = repo::draft(&state.db, pdf.space_id()).await?;
    let draft_files: Vec<DraftFile> = stored
        .draft_files()
        .into_iter()
        .map(|(path, o)| {
            let change = if files.iter().any(|f| f.path == path) {
                Change::Replaced
            } else {
                Change::Added
            };
            DraftFile {
                path,
                size: o.version.size,
                change,
            }
        })
        .collect();
    let deleted = row.as_ref().map(|r| r.deleted.clone()).unwrap_or_default();
    let draft = if draft_files.is_empty() && deleted.is_empty() {
        None
    } else {
        Some(DraftListing {
            files: draft_files,
            deleted,
            updated_at: row.as_ref().map(|r| r.updated_at),
            updated_by: repo::user_ref(&state.db, row.and_then(|r| r.updated_by)).await?,
        })
    };
    Ok(DesignListing {
        storage: storage.kind().to_string(),
        files,
        draft,
        quota: Quota {
            used: stored.usage(),
            limit: QUOTA_BYTES,
        },
    })
}

#[utoipa::path(
    get,
    path = "/api/pdf/design",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, body = DesignListing),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn design(
    State(state): State<AppState>,
    access: Read,
) -> Result<Json<DesignListing>, AppError> {
    let pdf = state.pdf.space(access.space())?;
    Ok(Json(listing(&state, &pdf).await?))
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
        Some(e) if e == "typ" || e == "toml" || e == "txt" => "text/plain; charset=utf-8",
        Some(e) if e == "svg" => "image/svg+xml",
        Some(e) if e == "png" => "image/png",
        Some(e) if e == "jpg" || e == "jpeg" => "image/jpeg",
        Some(e) if e == "ttf" => "font/ttf",
        Some(e) if e == "otf" => "font/otf",
        Some(e) if e == "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// A missing object (or a key the storage refuses) is an unknown file.
fn not_found(e: storage::Error) -> AppError {
    match e {
        storage::Error::NotFound(_) | storage::Error::InvalidKey(_) => AppError::NotFound,
        other => other.into(),
    }
}

#[utoipa::path(
    get,
    path = "/api/pdf/design/file",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    params(FileQuery),
    responses(
        (status = 200, description = "The file as an attachment", content_type = "application/octet-stream"),
        (status = 404, description = "No such file in that source", body = ErrorBody),
        (status = 422, description = "`source`: `invalid`", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn file(
    State(state): State<AppState>,
    access: Manage,
    ApiQuery(q): ApiQuery<FileQuery>,
) -> Result<Response, AppError> {
    if !safe_path(&q.path) {
        return Err(AppError::NotFound);
    }
    let pdf = state.pdf.space(access.space())?;
    let storage = pdf.storage();
    let stored = |key: String| async move { storage.get_stream(&key).await.map_err(not_found) };
    let default = || -> Result<Body, AppError> {
        if !design::embedded().contains_key(&q.path) {
            return Err(AppError::NotFound);
        }
        Ok(Body::from(design::embedded_bytes(&q.path)?))
    };
    let body = match q.source.as_deref().unwrap_or("live") {
        "live" => match stored(design::key(&q.path)).await {
            Ok(d) => Body::from_stream(d.stream),
            Err(AppError::NotFound) => default()?,
            Err(e) => return Err(e),
        },
        "draft" => Body::from_stream(stored(draft_key(&q.path)).await?.stream),
        "default" => default()?,
        _ => return Err(AppError::field("source", "invalid")),
    };
    let name = q.path.rsplit('/').next().unwrap_or(&q.path);
    let disposition = format!("attachment; filename=\"{}\"", safe_filename(name));
    Ok((
        [
            (header::CONTENT_TYPE, content_type(&q.path).to_string()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        body,
    )
        .into_response())
}

/// The upload path rule against the current default, live and draft paths.
fn check_path(path: &str, stored: &Stored) -> Result<(), AppError> {
    let embedded = design::embedded();
    let (live, draft) = (stored.live_files(), stored.draft_files());
    let existing = embedded
        .keys()
        .chain(live.keys())
        .chain(draft.keys())
        .map(String::as_str);
    if !valid_upload_path(path, existing) {
        return Err(AppError::field("path", "invalid"));
    }
    Ok(())
}

fn too_large() -> AppError {
    AppError::field("file", "too_large")
}

/// Read the body, refusing it as soon as it passes 10 MB (a declared
/// `Content-Length` over the limit is refused before reading anything).
async fn read_capped(headers: &HeaderMap, body: Body) -> Result<Bytes, AppError> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_FILE_BYTES) {
        return Err(too_large());
    }
    let mut stream = body.into_data_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::BadRequest(e.to_string()))?;
        if (bytes.len() + chunk.len()) as u64 > MAX_FILE_BYTES {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(bytes))
}

#[utoipa::path(
    put,
    path = "/api/pdf/design/draft/file",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    params(PathQuery),
    request_body(content_type = "application/octet-stream", description = "The raw file, at most 10 MB"),
    responses(
        (status = 200, description = "Stored in the draft; the design listing", body = DesignListing),
        (status = 409, description = "`design_busy` (another design change holds the lock)", body = ErrorBody),
        (status = 422, description = "`validation` (`path`: `invalid`; `file`: `too_large` / `invalid` — a non-UTF-8 `invoice.typ`), `quota_exceeded` (+ `detail`: the design's 50 MB or the space's storage quota)", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn put_file(
    State(state): State<AppState>,
    access: Manage,
    ApiQuery(q): ApiQuery<PathQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<DesignListing>, AppError> {
    let pdf = state.pdf.space(access.space())?;
    let storage = pdf.storage();
    // Path first, then the size: nothing is buffered for a refused upload.
    check_path(&q.path, &Stored::load(storage).await?)?;
    let bytes = read_capped(&headers, body).await?;
    if q.path == TEMPLATE && std::str::from_utf8(&bytes).is_err() {
        return Err(AppError::field("file", "invalid"));
    }
    let (txn, row) = locked(&state.db, access.space()).await?;
    let result = async {
        // Re-read under the lock: the quota and clashes count what is stored now.
        let stored = Stored::load(storage).await?;
        check_path(&q.path, &stored)?;
        let used = stored.usage();
        let replaced = stored
            .draft_files()
            .get(&q.path)
            .map_or(0, |o| o.version.size);
        if used.saturating_sub(replaced) + bytes.len() as u64 > QUOTA_BYTES {
            return Err(AppError::QuotaExceeded(format!(
                "{} / 50 MB",
                megabytes(used)
            )));
        }
        // Records the new design total, committed with the upload.
        let added = bytes.len() as u64;
        limits::check_design(&txn, access.space(), used, added, replaced).await?;
        storage.put(&draft_key(&q.path), bytes).await?;
        let deleted = row
            .deleted
            .iter()
            .filter(|p| **p != q.path)
            .cloned()
            .collect();
        repo::touch(&txn, &row, access.scope.user_id, deleted).await
    }
    .await;
    in_txn(txn, result).await?;
    Ok(Json(listing(&state, &pdf).await?))
}

#[utoipa::path(
    delete,
    path = "/api/pdf/design/draft/file",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    params(PathQuery),
    responses(
        (status = 200, description = "Draft file removed, or a live custom file marked for deletion; the design listing", body = DesignListing),
        (status = 404, description = "Neither a draft nor a live custom file", body = ErrorBody),
        (status = 409, description = "`design_busy`", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn delete_file(
    State(state): State<AppState>,
    access: Manage,
    ApiQuery(q): ApiQuery<PathQuery>,
) -> Result<Json<DesignListing>, AppError> {
    if !safe_path(&q.path) {
        return Err(AppError::NotFound);
    }
    let pdf = state.pdf.space(access.space())?;
    let (txn, row) = locked(&state.db, access.space()).await?;
    let result = async {
        let mut deleted = row.deleted.clone();
        let storage = pdf.storage();
        let key = draft_key(&q.path);
        if storage.head(&key).await?.is_some() {
            storage.delete(&key).await?;
        } else if storage.head(&design::key(&q.path)).await?.is_some() {
            if !deleted.contains(&q.path) {
                deleted.push(q.path.clone());
                deleted.sort();
            }
        } else {
            return Err(AppError::NotFound);
        }
        repo::touch(&txn, &row, access.scope.user_id, deleted).await
    }
    .await;
    in_txn(txn, result).await?;
    Ok(Json(listing(&state, &pdf).await?))
}

#[utoipa::path(
    delete,
    path = "/api/pdf/design/draft",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 204, description = "The draft is discarded"),
        (status = 409, description = "`design_busy`", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn discard(
    State(state): State<AppState>,
    access: Manage,
) -> Result<StatusCode, AppError> {
    let pdf = state.pdf.space(access.space())?;
    let (txn, row) = locked(&state.db, access.space()).await?;
    let result = async {
        pdf.storage().delete_prefix(DRAFT_PREFIX).await?;
        repo::touch(&txn, &row, access.scope.user_id, Vec::new()).await
    }
    .await;
    in_txn(txn, result).await?;
    refresh_usage(&state.db, access.space(), pdf.storage()).await;
    Ok(StatusCode::NO_CONTENT)
}
