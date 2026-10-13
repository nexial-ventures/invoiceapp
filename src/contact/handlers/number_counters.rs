use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::{Manage, Read};
use crate::contact::repo::number_counters::{self as repo, ContactSeries};
use crate::document::doc_type::DocType;
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::{ApiJson, ApiPath};
use crate::settings::handlers::number_series::{Counter, CounterInput};
use crate::time::current_year;

/// A customer's client sequence in one issued series.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContactNumberSeries {
    pub doc_type: DocType,
    /// `null` only in a `PUT` answer for a series without a client pattern.
    pub client_pattern: Option<String>,
    /// Newest year first.
    pub counters: Vec<Counter>,
    /// The customer's next number in the current year; `null` without a
    /// number code (or without a client pattern).
    pub next_number_preview: Option<String>,
}

impl From<ContactSeries> for ContactNumberSeries {
    fn from(s: ContactSeries) -> Self {
        Self {
            doc_type: s.doc_type,
            client_pattern: s.client_pattern,
            counters: s
                .counters
                .into_iter()
                .map(|c| Counter {
                    year: c.year,
                    last_number: c.last_number,
                })
                .collect(),
            next_number_preview: s.next_number_preview,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/contacts/{id}/number-counters",
    tag = "contacts",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Issued series with a client pattern", body = Vec<ContactNumberSeries>),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    access: Read,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Vec<ContactNumberSeries>>, AppError> {
    let items = repo::list(&state.db, access.space(), id, current_year()).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    put,
    path = "/api/contacts/{id}/number-counters/{docType}/{year}",
    tag = "contacts",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path), ("docType" = DocType, Path), ("year" = i32, Path)),
    request_body = CounterInput,
    responses(
        (status = 200, body = ContactNumberSeries),
        (status = 404, body = ErrorBody),
        (status = 422, description = "`year` / `lastNumber` out of range, `docType: not_issued`", body = ErrorBody),
    )
)]
pub async fn put(
    State(state): State<AppState>,
    access: Manage,
    ApiPath((id, doc_type, year)): ApiPath<(Uuid, DocType, i32)>,
    ApiJson(input): ApiJson<CounterInput>,
) -> Result<Json<ContactNumberSeries>, AppError> {
    let mut e = FieldErrors::new();
    if !(1..=9999).contains(&year) {
        e.add("year", "invalid");
    }
    let last_number = i32::try_from(input.last_number).ok().filter(|n| *n >= 0);
    if last_number.is_none() {
        e.add("lastNumber", "invalid");
    }
    e.into_result()?;
    let item = repo::set(
        &state.db,
        access.space(),
        id,
        doc_type,
        year,
        last_number.unwrap_or_default(),
        current_year(),
    )
    .await?;
    Ok(Json(item.into()))
}
