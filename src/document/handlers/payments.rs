use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::{Read, Write};
use crate::document::repo::payments as repo;
use crate::document::service::add_payment;
use crate::document::wire::dto::{Payment, PaymentInput};
use crate::error::{AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath};

#[utoipa::path(
    get,
    path = "/api/documents/{id}/payments",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Vec<Payment>), (status = 404, body = ErrorBody))
)]
pub async fn list(
    State(state): State<AppState>,
    access: Read,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Vec<Payment>>, AppError> {
    let rows = repo::list(&state.db, access.space(), id).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    post,
    path = "/api/documents/{id}/payments",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = PaymentInput,
    responses(
        (status = 201, body = Payment),
        (status = 404, body = ErrorBody),
        (status = 409, description = "`invalid_state` (not issued, or a DDPP)", body = ErrorBody),
        (status = 422, description = "Validation failed; `exchangeRate: required` when a DDPP needs a rate ČNB cannot give", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    access: Write,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<PaymentInput>,
) -> Result<(StatusCode, Json<Payment>), AppError> {
    Ok((
        StatusCode::CREATED,
        Json(add_payment(&state, access.space(), id, input).await?),
    ))
}

#[utoipa::path(
    delete,
    path = "/api/documents/{id}/payments/{paymentId}",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path), ("paymentId" = Uuid, Path)),
    responses(
        (status = 204),
        (status = 404, body = ErrorBody),
        (status = 409, description = "`invalid_state` (not issued) / `advance_settled` (its DDPP is deducted by an issued invoice)", body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    access: Write,
    ApiPath((id, payment_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    repo::delete(&state.db, access.space(), id, payment_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
