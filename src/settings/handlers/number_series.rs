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
    /// Issued series only: the pattern for customers with a number code
    /// (exactly one `{C}`); `null` = everyone uses `pattern`.
    pub client_pattern: Option<String>,
    /// Newest year first.
    pub counters: Vec<Counter>,
    /// The number the next allocation of the global series in the current
    /// year would get (past the counter and every number held).
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
pub struct ClientPatternInput {
    /// Empty / `null` clears it.
    #[serde(default)]
    pub client_pattern: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CounterInput {
    pub last_number: i64,
}

impl NumberSeries {
    pub fn build(s: Series) -> Self {
        Self {
            doc_type: s.doc_type,
            pattern: s.pattern,
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
    let series = repo::number_series::list(&state.db, access.space(), year).await?;
    Ok(Json(series.into_iter().map(NumberSeries::build).collect()))
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
    let year = current_year();
    let s = repo::number_series::set_pattern(&state.db, access.space(), doc_type, pattern, year)
        .await?;
    Ok(Json(NumberSeries::build(s)))
}

#[utoipa::path(
    put,
    path = "/api/settings/number-series/{docType}/client-pattern",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    params(("docType" = DocType, Path)),
    request_body = ClientPatternInput,
    responses(
        (status = 200, body = NumberSeries),
        (status = 404, body = ErrorBody),
        (status = 422, description = "`clientPattern`: `invalid_pattern` / `not_issued` / `duplicate`", body = ErrorBody),
    )
)]
pub async fn put_client_pattern(
    State(state): State<AppState>,
    access: Manage,
    ApiPath(doc_type): ApiPath<DocType>,
    ApiJson(input): ApiJson<ClientPatternInput>,
) -> Result<Json<NumberSeries>, AppError> {
    let pattern = client_pattern(doc_type, input.client_pattern.as_deref())?;
    let year = current_year();
    let s =
        repo::number_series::set_client_pattern(&state.db, access.space(), doc_type, pattern, year)
            .await?;
    Ok(Json(NumberSeries::build(s)))
}

/// Trimmed; empty clears. Only the seven issued series take one.
fn client_pattern(doc_type: DocType, input: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(p) = input.map(str::trim).filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    if !doc_type.is_document_type() {
        return Err(AppError::field("clientPattern", "not_issued"));
    }
    Pattern::parse_client(p).map_err(|_| AppError::field("clientPattern", "invalid_pattern"))?;
    Ok(Some(p.to_string()))
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
/// Any value >= 0: allocation skips numbers already held anyway.
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
        current_year(),
    )
    .await?;
    Ok(Json(NumberSeries::build(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_pattern_input() {
        let ok = client_pattern(DocType::Invoice, Some(" {C}-{YYYY}{NNN} "));
        assert_eq!(ok.ok().flatten().as_deref(), Some("{C}-{YYYY}{NNN}"));
        assert!(matches!(
            client_pattern(DocType::Invoice, Some("  ")),
            Ok(None)
        ));
        assert!(matches!(client_pattern(DocType::Received, None), Ok(None)));
        let field = |r: Result<Option<String>, AppError>| match r {
            Err(AppError::Validation(f)) => f.get("clientPattern"),
            _ => None,
        };
        assert_eq!(
            field(client_pattern(DocType::Invoice, Some("{YYYY}{NNN}"))),
            Some("invalid_pattern")
        );
        assert_eq!(
            field(client_pattern(DocType::Received, Some("{C}{YYYY}{NNN}"))),
            Some("not_issued")
        );
    }
}
