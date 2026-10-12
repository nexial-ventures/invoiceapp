//! `GET` / `PUT /api/settings/accounting`.

use axum::Json;
use axum::extract::State;

use super::repo;
use super::settings::{AccountingResponse, AccountingUpdate};
use crate::app::AppState;
use crate::auth::{Manage, Read};
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiJson;

#[utoipa::path(
    get,
    path = "/api/settings/accounting",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, body = AccountingResponse, description = "Both sections (`pohoda`, `money`), every direction × exported type row present (missing ones all null), and the code length `limits`"))
)]
pub async fn get(
    State(state): State<AppState>,
    access: Read,
) -> Result<Json<AccountingResponse>, AppError> {
    Ok(Json(repo::get(&state.db, access.space()).await?.into()))
}

#[utoipa::path(
    put,
    path = "/api/settings/accounting",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    request_body(content = AccountingUpdate, description = "A section present replaces that section (`{}` clears it); an absent one is kept"),
    responses(
        (status = 200, body = AccountingResponse),
        (status = 422, description = "`{pohoda|money}.ico`: `invalid_ico`; `{pohoda|money}.codes.N.direction` / `docType`: `invalid`, `docType`: `duplicate`; `….classificationVatNonDeductible`: `invalid` on an issued row; `pohoda.codes.N.{accounting|classificationVat|classificationVatNonDeductible|numberSeries}`: `too_long` (> 19); `money.codes.N.{accounting|classificationVat|classificationVatNonDeductible}`: `too_long` (> 10), `money.codes.N.numberSeries`: `too_long` (> 5)", body = ErrorBody),
    )
)]
pub async fn put(
    State(state): State<AppState>,
    access: Manage,
    ApiJson(input): ApiJson<AccountingUpdate>,
) -> Result<Json<AccountingResponse>, AppError> {
    let update = input.validate()?;
    Ok(Json(
        repo::update(&state.db, access.space(), update)
            .await?
            .into(),
    ))
}
