//! `POST /api/import/isdoc/preview` and `…/confirm`.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::extract::multipart::MultipartRejection;
use axum::extract::{Multipart, State};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::analyze::{self, Analyzed, Ready, Status};
use super::parse::Code;
use crate::app::AppState;
use crate::auth::Write;
use crate::cnb;
use crate::document::repo::issue::Rate;
use crate::error::{AppError, ErrorBody};
use crate::import::category::CategoryRef;
use crate::import::store::{self, Options};
use crate::import::wire::{self, Confirmed, PreviewEntry};
use crate::import::{form, lookup};
use crate::pdf::PdfService;
use crate::settings::doc_type::RECEIVED;
use crate::settings::entity::category;
use crate::space::{SpaceId, limits};
use crate::time::today;

#[derive(Debug, Serialize, ToSchema)]
pub struct Preview {
    pub entries: Vec<PreviewEntry>,
}

/// The `options` part of `confirm` (a text field holding JSON).
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct OptionsInput {
    pub selected: Vec<String>,
    /// Default `true`.
    pub mark_paid: Option<bool>,
    pub category_id: Option<Uuid>,
    /// Default `true`.
    pub vat_deductible: Option<bool>,
}

fn entry(a: &Analyzed) -> PreviewEntry {
    match &a.outcome {
        Err(code) => PreviewEntry::error(a.key.clone(), code),
        Ok(r) => PreviewEntry {
            has_pdf: r.pdf.is_some(),
            ..PreviewEntry::planned(a.key.clone(), &r.plan, &r.checked, r.warnings.clone())
        },
    }
}

#[utoipa::path(
    post,
    path = "/api/import/isdoc/preview",
    tag = "import",
    security(("cookie" = []), ("bearer" = [])),
    request_body(content_type = "multipart/form-data", description = "`files` parts: `.isdoc`, `.isdocx` or `.zip` (≤ 50 MiB together)"),
    responses(
        (status = 200, body = Preview),
        (status = 413, description = "`too_large`", body = ErrorBody),
        (status = 422, description = "`files`: `required` / `too_many` / `too_large`", body = ErrorBody),
    )
)]
pub async fn preview(
    State(state): State<AppState>,
    access: Write,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Preview>, AppError> {
    let (files, _) = form::read(form, "files").await?;
    let analyzed = analyze::analyze(&state.db, access.space(), files, None).await?;
    Ok(Json(Preview {
        entries: analyzed.iter().map(entry).collect(),
    }))
}

/// An expense category that can be assigned.
async fn check_category(
    state: &AppState,
    space: SpaceId,
    id: Option<Uuid>,
) -> Result<(), AppError> {
    let Some(id) = id else { return Ok(()) };
    let found = category::Entity::find_by_id(id)
        .filter(category::Column::SpaceId.eq(space))
        .one(&state.db)
        .await?;
    match found {
        Some(c) if c.kind == "expense" && c.active => Ok(()),
        Some(c) if c.kind == "expense" => Err(AppError::field("categoryId", "inactive")),
        _ => Err(AppError::field("categoryId", "invalid")),
    }
}

/// The ISDOC rate (manual), else ČNB for the tax point (`rate_unavailable`).
async fn rate(state: &AppState, r: &Ready) -> Result<Rate, Code> {
    let p = &r.plan;
    if p.currency == "CZK" {
        return Ok(Rate {
            rate: None,
            date: None,
            source: None,
        });
    }
    if let Some(rate) = p.rate {
        return Ok(Rate {
            rate: Some(rate),
            date: None,
            source: Some("manual"),
        });
    }
    match cnb::repo::rate(&state.db, &state.cnb, &p.currency, p.rate_date(), today()).await {
        Ok(r) => Ok(Rate {
            rate: Some(r.rate),
            date: Some(r.date),
            source: Some("cnb"),
        }),
        Err(e) => {
            tracing::warn!(error = %e, "ISDOC import: no ČNB rate");
            Err("rate_unavailable")
        }
    }
}

/// The batch options of `confirm`, as one entry's [`Options`].
fn entry_options(input: &OptionsInput, r: &Ready) -> Options {
    let p = &r.plan;
    Options {
        paid_on: input
            .mark_paid
            .unwrap_or(true)
            .then(|| p.due_date.unwrap_or(p.issue_date)),
        category: input
            .category_id
            .filter(|_| p.direction == RECEIVED)
            .map(CategoryRef::Id),
        vat_deductible: input.vat_deductible.unwrap_or(true),
    }
}

async fn import_one(
    state: &AppState,
    pdf: &PdfService,
    r: &Ready,
    opts: &Options,
) -> Result<Uuid, Code> {
    let rate = rate(state, r).await?;
    store::import(&state.db, pdf, &r.plan, r.pdf.clone(), &rate, opts)
        .await
        .map_err(wire::failure)
}

#[utoipa::path(
    post,
    path = "/api/import/isdoc/confirm",
    tag = "import",
    security(("cookie" = []), ("bearer" = [])),
    request_body(content_type = "multipart/form-data", description = "The same `files` as the preview plus an `options` text part holding JSON (`OptionsInput`)"),
    responses(
        (status = 200, body = Confirmed),
        (status = 413, description = "`too_large`", body = ErrorBody),
        (status = 422, description = "`files` / `options` / `categoryId`; `quota_exceeded` (+ `detail`) when the selected entries' PDFs do not fit the space's storage quota", body = ErrorBody),
    )
)]
pub async fn confirm(
    State(state): State<AppState>,
    access: Write,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Confirmed>, AppError> {
    let (files, raw) = form::read(form, "files").await?;
    let input: OptionsInput = form::options(raw)?;
    let space = access.space();
    check_category(&state, space, input.category_id).await?;
    let pdf = state.pdf.space(space)?;
    let selected: HashSet<String> = input.selected.iter().cloned().collect();
    let analyzed = analyze::analyze(&state.db, space, files, Some(&selected)).await?;
    let mut todo: Vec<(&str, &Ready)> = analyzed
        .iter()
        .filter(|a| selected.contains(&a.key))
        .filter_map(|a| match &a.outcome {
            Ok(r) if r.checked.status == Status::Ok => Some((a.key.as_str(), r)),
            _ => None,
        })
        .collect();
    todo.sort_by_key(|(_, r)| lookup::rank(r.plan.doc_type));
    // All or nothing up front; a concurrent upload can still make a later
    // entry fail with `quota_exceeded`.
    let pdf_bytes = todo
        .iter()
        .filter_map(|(_, r)| r.pdf.as_ref())
        .map(|b| b.len() as u64)
        .sum();
    limits::check_room(&state.db, space, pdf_bytes).await?;
    let mut done: HashMap<&str, Result<Uuid, Code>> = HashMap::new();
    for (key, r) in todo {
        let opts = entry_options(&input, r);
        done.insert(key, import_one(&state, &pdf, r, &opts).await);
    }
    Ok(Json(wire::confirmed(
        analyzed.iter().map(|a| a.key.as_str()),
        &done,
    )))
}
