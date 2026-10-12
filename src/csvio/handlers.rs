//! `POST /api/import/csv/preview`, `…/confirm` and `GET …/sample`.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::extract::multipart::MultipartRejection;
use axum::extract::{Multipart, State};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::analyze::{self, Entry, Ready};
use super::write;
use crate::app::AppState;
use crate::auth::{Read, Write};
use crate::document::line::Direction;
use crate::document::repo::issue::Rate;
use crate::error::{AppError, ErrorBody};
use crate::import::category::CategoryRef;
use crate::import::check::Status;
use crate::import::model::Code;
use crate::import::store::{self, Options};
use crate::import::wire::{self, Confirmed, Counterparty, PreviewEntry};
use crate::import::{form, lookup};
use crate::pdf::PdfService;
use crate::settings::repo::vat_rates;
use sea_orm::DatabaseConnection;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreviewEntry {
    #[serde(flatten)]
    pub entry: PreviewEntry,
    /// The 1-based line / sheet row (the header is 1).
    pub row: u32,
    /// The column a row error refers to.
    pub field: Option<String>,
    /// `existing` | `new`.
    pub category_match: Option<&'static str>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CsvPreview {
    pub entries: Vec<CsvPreviewEntry>,
}

/// The `options` part of `confirm` (a text field holding JSON).
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct CsvOptionsInput {
    pub selected: Vec<String>,
}

/// Exactly one `file` part.
async fn read_file(
    form: Result<Multipart, MultipartRejection>,
) -> Result<(Vec<u8>, Option<String>), AppError> {
    let (mut files, options) = form::read(form, "file").await?;
    match files.pop() {
        Some(f) if files.is_empty() => Ok((f.bytes, options)),
        _ => Err(AppError::field("file", "invalid")),
    }
}

fn entry(e: &Entry) -> CsvPreviewEntry {
    match &e.outcome {
        Ok(r) => CsvPreviewEntry {
            entry: PreviewEntry::planned(
                e.key.clone(),
                &r.mapped.plan,
                &r.checked,
                r.warnings.clone(),
            ),
            row: e.line,
            field: None,
            category_match: r.category_match,
        },
        Err((err, raw)) => CsvPreviewEntry {
            entry: PreviewEntry {
                direction: raw.direction.map(Direction::as_str),
                doc_type: raw.doc_type,
                number: raw.number.clone(),
                counterparty: raw.counterparty.as_ref().map(|(name, ico)| Counterparty {
                    name: name.clone(),
                    ico: ico.clone(),
                }),
                ..PreviewEntry::error(e.key.clone(), err.code)
            },
            row: e.line,
            field: err.field.clone(),
            category_match: None,
        },
    }
}

#[utoipa::path(
    post,
    path = "/api/import/csv/preview",
    tag = "import",
    security(("cookie" = []), ("bearer" = [])),
    request_body(content_type = "multipart/form-data", description = "One `file` part: `.csv`, `.txt` or `.xlsx` (≤ 50 MiB)"),
    responses(
        (status = 200, body = CsvPreview),
        (status = 413, description = "`too_large`", body = ErrorBody),
        (status = 422, description = "`file`: `required` / `invalid` / `empty` / `too_many` / `too_large` / `missing_column` / `invalid_column` (`detail` = the column)", body = ErrorBody),
    )
)]
pub async fn preview(
    State(state): State<AppState>,
    access: Write,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<CsvPreview>, AppError> {
    let (bytes, _) = read_file(form).await?;
    let entries = analyze::analyze(&state.db, access.space(), bytes, None).await?;
    Ok(Json(CsvPreview {
        entries: entries.iter().map(entry).collect(),
    }))
}

async fn import_one(pdf: &PdfService, db: &DatabaseConnection, r: &Ready) -> Result<Uuid, Code> {
    let m = &r.mapped;
    let rate = Rate {
        rate: m.plan.rate,
        date: None,
        source: m.plan.rate.map(|_| "manual"),
    };
    let opts = Options {
        paid_on: m.paid_date,
        category: m.category.clone().map(CategoryRef::Name),
        vat_deductible: m.vat_deductible,
    };
    store::import(db, pdf, &m.plan, None, &rate, &opts)
        .await
        .map_err(wire::failure)
}

#[utoipa::path(
    post,
    path = "/api/import/csv/confirm",
    tag = "import",
    security(("cookie" = []), ("bearer" = [])),
    request_body(content_type = "multipart/form-data", description = "The same `file` as the preview plus an `options` text part holding JSON (`CsvOptionsInput`)"),
    responses(
        (status = 200, body = Confirmed),
        (status = 413, description = "`too_large`", body = ErrorBody),
        (status = 422, description = "`file` / `options`", body = ErrorBody),
    )
)]
pub async fn confirm(
    State(state): State<AppState>,
    access: Write,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Confirmed>, AppError> {
    let (bytes, raw) = read_file(form).await?;
    let input: CsvOptionsInput = form::options(raw)?;
    let selected: HashSet<String> = input.selected.into_iter().collect();
    let entries = analyze::analyze(&state.db, access.space(), bytes, Some(&selected)).await?;
    let pdf = state.pdf.space(access.space())?;
    let mut todo: Vec<(&str, &Ready)> = entries
        .iter()
        .filter(|e| selected.contains(&e.key))
        .filter_map(|e| match &e.outcome {
            Ok(r) if r.checked.status == Status::Ok => Some((e.key.as_str(), r)),
            _ => None,
        })
        .collect();
    todo.sort_by_key(|(_, r)| lookup::rank(r.mapped.plan.doc_type));
    let mut done: HashMap<&str, Result<Uuid, Code>> = HashMap::new();
    for (key, r) in todo {
        done.insert(key, import_one(&pdf, &state.db, r).await);
    }
    Ok(Json(wire::confirmed(
        entries.iter().map(|e| e.key.as_str()),
        &done,
    )))
}

#[utoipa::path(
    get,
    path = "/api/import/csv/sample",
    tag = "import",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, content_type = "text/csv", description = "`import-sample.csv`: the header with the current rate columns and three example rows"))
)]
pub async fn sample(State(state): State<AppState>, access: Read) -> Result<Response, AppError> {
    let rates: Vec<_> = vat_rates::list(&state.db, access.space())
        .await?
        .into_iter()
        .map(|r| r.rate)
        .collect();
    let body = write::sample(&rates)?;
    Ok(crate::download::attachment(
        "text/csv; charset=utf-8",
        "import-sample.csv",
        body,
    ))
}
