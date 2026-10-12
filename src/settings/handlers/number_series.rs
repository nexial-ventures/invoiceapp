use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::auth::{Manage, Read};
use crate::document::doc_type::DocType;
use crate::error::{AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath};
use crate::settings::pattern::Pattern;
use crate::settings::repo;
use crate::settings::repo::number_series::Series;
use crate::time::current_year;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NumberSeries {
    pub doc_type: DocType,
    pub pattern: String,
    /// Newest year first.
    pub counters: Vec<Counter>,
    /// The number the next allocation in the current year would get.
    pub next_number_preview: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Counter {
    pub year: i32,
    pub last_number: i32,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatternInput {
    #[serde(default)]
    pub pattern: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CounterInput {
    pub last_number: i64,
}

impl NumberSeries {
    pub fn build(s: Series, year: i32) -> Self {
        let last = s
            .counters
            .iter()
            .find(|c| c.year == year)
            .map_or(0, |c| c.last_number);
        // Stored patterns are validated on write; an unparsable one shows as-is.
        let next_number_preview = Pattern::parse(&s.pattern)
            .map(|p| p.format(year, i64::from(last) + 1))
            .unwrap_or_else(|_| s.pattern.clone());
        Self {
            doc_type: s.doc_type,
            pattern: s.pattern,
            counters: s
                .counters
                .into_iter()
                .map(|c| Counter {
                    year: c.year,
                    last_number: c.last_number,
                })
                .collect(),
            next_number_preview,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/settings/number-series",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, body = Vec<NumberSeries>))
)]
pub async fn list(
    State(state): State<AppState>,
    access: Read,
) -> Result<Json<Vec<NumberSeries>>, AppError> {
    let year = current_year();
    let series = repo::number_series::list(&state.db, access.space()).await?;
    Ok(Json(
        series
            .into_iter()
            .map(|s| NumberSeries::build(s, year))
            .collect(),
    ))
}

#[utoipa::path(
    put,
    path = "/api/settings/number-series/{docType}",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    params(("docType" = DocType, Path)),
    request_body = PatternInput,
    responses(
        (status = 200, body = NumberSeries),
        (status = 404, body = ErrorBody),
        (status = 422, description = "`pattern`: `invalid_pattern` / `duplicate`", body = ErrorBody),
    )
)]
pub async fn put_pattern(
    State(state): State<AppState>,
    access: Manage,
    ApiPath(doc_type): ApiPath<DocType>,
    ApiJson(input): ApiJson<PatternInput>,
) -> Result<Json<NumberSeries>, AppError> {
    let pattern = input.pattern.trim().to_string();
    Pattern::parse(&pattern).map_err(|_| AppError::field("pattern", "invalid_pattern"))?;
    let s = repo::number_series::set_pattern(&state.db, access.space(), doc_type, pattern).await?;
    Ok(Json(NumberSeries::build(s, current_year())))
}

#[utoipa::path(
    put,
    path = "/api/settings/number-series/{docType}/counters/{year}",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    params(("docType" = DocType, Path), ("year" = i32, Path)),
    request_body = CounterInput,
    responses(
        (status = 200, body = NumberSeries),
        (status = 404, body = ErrorBody),
        (status = 422, description = "`year` / `lastNumber` out of range", body = ErrorBody),
    )
)]
pub async fn put_counter(
    State(state): State<AppState>,
    access: Manage,
    ApiPath((doc_type, year)): ApiPath<(DocType, i32)>,
    ApiJson(input): ApiJson<CounterInput>,
) -> Result<Json<NumberSeries>, AppError> {
    let mut e = crate::error::FieldErrors::new();
    if !(1..=9999).contains(&year) {
        e.add("year", "invalid");
    }
    let last_number = i32::try_from(input.last_number).ok().filter(|n| *n >= 0);
    if last_number.is_none() {
        e.add("lastNumber", "invalid");
    }
    e.into_result()?;
    let s = repo::number_series::set_counter(
        &state.db,
        access.space(),
        doc_type,
        year,
        last_number.unwrap_or_default(),
    )
    .await?;
    Ok(Json(NumberSeries::build(s, current_year())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::entity::number_series_counter as counter;

    #[test]
    fn preview_uses_current_year_counter() {
        let series = |counters| Series {
            doc_type: DocType::Invoice,
            pattern: "{YYYY}{NNNN}".into(),
            counters,
        };
        let c = |year, last_number| counter::Model {
            series_id: uuid::Uuid::nil(),
            year,
            last_number,
        };
        assert_eq!(
            NumberSeries::build(series(vec![]), 2026).next_number_preview,
            "20260001"
        );
        assert_eq!(
            NumberSeries::build(series(vec![c(2026, 41), c(2025, 99)]), 2026).next_number_preview,
            "20260042"
        );
        assert_eq!(
            NumberSeries::build(series(vec![c(2025, 99)]), 2026).next_number_preview,
            "20260001"
        );
    }
}
