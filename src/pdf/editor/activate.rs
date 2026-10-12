//! Activation, restore, reset and the version history.
//!
//! Activate / restore render their check **before** taking the lock (a
//! render must not hold a pooled connection and the row lock), then lock
//! with `NOWAIT`, verify that what was checked is still what would be
//! applied, and apply. Storage objects that rows stop pointing at are only
//! deleted after the commit.

use axum::Json;
use axum::extract::State;
use sea_orm::DatabaseTransaction;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::preview::SampleSource;
use super::{
    DRAFT_PREFIX, Version, VersionFile, files, in_txn, locked, refresh_usage, repo, usage,
    version_prefix,
};
use crate::app::AppState;
use crate::auth::Manage;
use crate::auth::ratelimit::DESIGN_SPACE;
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiPath;
use crate::pdf::PdfService;
use crate::pdf::design::{self, Layers};
use crate::pdf::format::Locale;
use crate::storage::DESIGN_PREFIX;

#[derive(Debug, Serialize, ToSchema)]
pub struct VersionResponse {
    /// The version just recorded (`current: true`).
    pub version: Version,
}

/// Render the sample with `layers` in both locales (company and bank read
/// once); a template failure is `design_invalid` with the locale in front.
async fn validate(state: &AppState, pdf: &PdfService, layers: Layers<'_>) -> Result<(), AppError> {
    let source = SampleSource::load(state, pdf.space_id()).await?;
    for locale in [Locale::Cs, Locale::En] {
        match source.render(pdf, locale, layers).await {
            Ok(_) => {}
            Err(AppError::PdfRenderFailed(m)) => {
                return Err(AppError::DesignInvalid(format!("{}: {m}", locale.as_str())));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn reserve(state: &AppState, pdf: &PdfService) -> Result<(), AppError> {
    let key = pdf.space_id().to_string();
    state.limiter.reserve(&[(DESIGN_SPACE, &key)])
}

/// A recorded version and the pruned versions whose objects go after the
/// commit.
struct Recorded {
    version: Version,
    pruned: Vec<Uuid>,
}

/// Under the lock: collect orphaned version objects, snapshot the live set
/// (not for a reset), insert the row and prune rows beyond ten.
async fn record(
    txn: &DatabaseTransaction,
    pdf: &PdfService,
    kind: &str,
    user: Uuid,
) -> Result<Recorded, AppError> {
    let (storage, space) = (pdf.storage(), pdf.space_id());
    files::collect_orphans(storage, &repo::version_ids(txn, space).await?).await;
    let id = Uuid::new_v4();
    let files = if kind == "default" {
        Vec::new()
    } else {
        files::snapshot(storage, id).await?
    };
    if let Err(e) = repo::insert_version(txn, space, id, kind, user, &files).await {
        files::remove_prefix(storage, &version_prefix(id)).await;
        return Err(e);
    }
    let pruned = repo::prune(txn, space).await?;
    let version = repo::versions(txn, space)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("the new design version is not listed"))?;
    Ok(Recorded { version, pruned })
}

/// Commit, then delete the pruned versions' objects and drop the cached
/// files of the space (the next render lists the new live set).
async fn finish(
    pdf: &PdfService,
    txn: DatabaseTransaction,
    result: Result<Recorded, AppError>,
) -> Result<Json<VersionResponse>, AppError> {
    let recorded = in_txn(txn, result).await;
    // Also on failure: the live files may have changed before it.
    pdf.invalidate_design();
    let recorded = recorded?;
    files::remove_versions(pdf.storage(), &recorded.pruned).await;
    Ok(Json(VersionResponse {
        version: recorded.version,
    }))
}

#[utoipa::path(
    post,
    path = "/api/pdf/design/activate",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The draft is live, recorded as an `activation` version; the draft is cleared", body = VersionResponse),
        (status = 409, description = "`design_busy`; `draft_changed` (the draft changed during the check) — nothing changed", body = ErrorBody),
        (status = 422, description = "`draft_empty`; `design_invalid` (+ `detail` `<locale>: <message>`) — nothing changed", body = ErrorBody),
        (status = 429, description = "`rate_limited` (10 activations / restores per hour per space)", body = ErrorBody),
        (status = 502, description = "`pdf_render_failed` (mdcast error that is not the template's)", body = ErrorBody),
        (status = 503, description = "`pdf_unavailable`, `storage_unavailable` (the draft is kept)", body = ErrorBody),
    )
)]
pub async fn activate(
    State(state): State<AppState>,
    access: Manage,
) -> Result<Json<VersionResponse>, AppError> {
    let (space, user) = (access.space(), access.scope.user_id);
    let pdf = state.pdf.space(space)?;
    let storage = pdf.storage();
    let checked = repo::draft(&state.db, space).await?;
    let checked_deleted = checked
        .as_ref()
        .map(|r| r.deleted.clone())
        .unwrap_or_default();
    let checked_revision = checked.as_ref().map_or(0, |r| r.revision);
    let draft = design::stored(storage, DRAFT_PREFIX).await?;
    if draft.is_empty() && checked_deleted.is_empty() {
        return Err(AppError::DraftEmpty);
    }
    reserve(&state, &pdf)?;
    let layers = Layers {
        base: DESIGN_PREFIX,
        deleted: &checked_deleted,
        top: Some(DRAFT_PREFIX),
    };
    validate(&state, &pdf, layers).await?;

    let (txn, row) = locked(&state.db, space).await?;
    let result = async {
        let unchanged = row.revision == checked_revision
            && row.deleted == checked_deleted
            && design::stored(storage, DRAFT_PREFIX).await? == draft;
        if !unchanged {
            return Err(AppError::DraftChanged);
        }
        files::apply_draft(storage, &draft, &row.deleted).await?;
        let recorded = record(&txn, &pdf, "activation", user).await?;
        // Live + the new version grew the design bytes: recorded before the commit.
        usage::record_in(&txn, space, storage).await?;
        repo::touch(&txn, &row, user, Vec::new()).await?;
        Ok(recorded)
    }
    .await;
    let response = finish(&pdf, txn, result).await?;
    // Already live; a file re-uploaded meanwhile is kept.
    files::remove_applied(storage, &draft).await;
    refresh_usage(&state.db, space, storage).await;
    Ok(response)
}

#[utoipa::path(
    get,
    path = "/api/pdf/design/versions",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "Newest first; the first one is `current`", body = [Version]),
    )
)]
pub async fn versions(
    State(state): State<AppState>,
    access: Manage,
) -> Result<Json<Vec<Version>>, AppError> {
    Ok(Json(repo::versions(&state.db, access.space()).await?))
}

/// The version's file list, `version_incomplete` when its stored objects
/// are missing or differ in size, 404 when there is no such version.
async fn complete_version(
    db: &impl sea_orm::ConnectionTrait,
    pdf: &PdfService,
    id: Uuid,
) -> Result<Vec<VersionFile>, AppError> {
    let row = repo::version(db, pdf.space_id(), id)
        .await?
        .ok_or(AppError::NotFound)?;
    let files: Vec<VersionFile> = serde_json::from_value(row.files)
        .map_err(|e| anyhow::anyhow!("design_versions.files: {e}"))?;
    if !files::complete(pdf.storage(), id, &files).await? {
        return Err(AppError::VersionIncomplete);
    }
    Ok(files)
}

#[utoipa::path(
    post,
    path = "/api/pdf/design/versions/{id}/restore",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "The version's files are live (others deleted), recorded as a `revert` version; the draft is untouched", body = VersionResponse),
        (status = 404, body = ErrorBody),
        (status = 409, description = "`design_busy`; `version_incomplete` (its stored files are missing or differ) — nothing changed", body = ErrorBody),
        (status = 422, description = "`design_invalid` (+ `detail`) — nothing changed", body = ErrorBody),
        (status = 429, description = "`rate_limited`", body = ErrorBody),
        (status = 502, description = "`pdf_render_failed`", body = ErrorBody),
        (status = 503, description = "`pdf_unavailable`, `storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn restore(
    State(state): State<AppState>,
    access: Manage,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<VersionResponse>, AppError> {
    let pdf = state.pdf.space(access.space())?;
    complete_version(&state.db, &pdf, id).await?;
    reserve(&state, &pdf)?;
    let prefix = version_prefix(id);
    let layers = Layers {
        base: &prefix,
        deleted: &[],
        top: None,
    };
    validate(&state, &pdf, layers).await?;

    let (txn, _row) = locked(&state.db, access.space()).await?;
    let result = async {
        // Pruned meanwhile → 404; its objects re-checked under the lock.
        let files = complete_version(&txn, &pdf, id).await?;
        files::replace_live(pdf.storage(), id, &files).await?;
        let recorded = record(&txn, &pdf, "revert", access.scope.user_id).await?;
        usage::record_in(&txn, access.space(), pdf.storage()).await?;
        Ok(recorded)
    }
    .await;
    finish(&pdf, txn, result).await
}

#[utoipa::path(
    post,
    path = "/api/pdf/design/reset",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "Every live custom file deleted (the embedded default applies), recorded as a `default` version", body = VersionResponse),
        (status = 409, description = "`design_busy`", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn reset(
    State(state): State<AppState>,
    access: Manage,
) -> Result<Json<VersionResponse>, AppError> {
    let pdf = state.pdf.space(access.space())?;
    let (txn, _row) = locked(&state.db, access.space()).await?;
    let result = async {
        pdf.storage().delete_prefix(DESIGN_PREFIX).await?;
        record(&txn, &pdf, "default", access.scope.user_id).await
    }
    .await;
    let response = finish(&pdf, txn, result).await?;
    refresh_usage(&state.db, access.space(), pdf.storage()).await;
    Ok(response)
}
