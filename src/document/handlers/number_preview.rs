use axum::Json;
use axum::extract::State;
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::Read;
use crate::document::doc_type::DocType;
use crate::document::repo::context;
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiQuery;
use crate::settings::repo::numbering::{self, NumberSource};
use crate::time::today;

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct NumberPreviewQuery {
    /// An issued document type; default `invoice`.
    pub doc_type: Option<String>,
    /// The customer: their number code picks the client sequence.
    pub contact_id: Option<Uuid>,
    /// Its year picks the counter; default today.
    pub issue_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NumberPreview {
    pub number: String,
    pub source: NumberSource,
}

#[utoipa::path(
    get,
    path = "/api/documents/number-preview",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(NumberPreviewQuery),
    responses(
        (status = 200, description = "The number an issue would allocate now (no counter moves)", body = NumberPreview),
        (status = 422, description = "`docType` / `contactId`: `invalid`", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    access: Read,
    ApiQuery(q): ApiQuery<NumberPreviewQuery>,
) -> Result<Json<NumberPreview>, AppError> {
    let space = access.space();
    let doc_type = match q.doc_type.as_deref().map(str::trim) {
        None | Some("") => DocType::Invoice,
        Some(s) => DocType::parse_document(s).ok_or(AppError::field("docType", "invalid"))?,
    };
    let customer = match q.contact_id {
        Some(id) => Some(
            context::contact(&state.db, space, id)
                .await?
                .ok_or(AppError::field("contactId", "invalid"))?,
        ),
        None => None,
    };
    let year = q.issue_date.unwrap_or_else(today).year();
    let (number, source) =
        numbering::preview(&state.db, space, doc_type, year, customer.as_ref()).await?;
    Ok(Json(NumberPreview { number, source }))
}
