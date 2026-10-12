use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use super::received;
use crate::app::AppState;
use crate::auth::{Read, Write};
use crate::document::line::Direction;
use crate::document::repo::{query, received as received_repo, write};
use crate::document::service::{compute_totals, create_draft, fetch, list_page, update_draft};
use crate::document::wire::compute_input::ComputeInput;
use crate::document::wire::dto::{Computed, Document, DocumentList, ListQuery};
use crate::document::wire::input::DocumentInput;
use crate::error::{AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, ApiQuery, from_value};

#[utoipa::path(
    get,
    path = "/api/documents",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(ListQuery),
    responses((status = 200, body = DocumentList))
)]
pub async fn list(
    State(state): State<AppState>,
    access: Read,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<DocumentList>, AppError> {
    let (q, limit, offset) = crate::paging::ListQuery {
        q: query.q.clone(),
        limit: query.limit,
        offset: query.offset,
    }
    .normalized();
    Ok(Json(
        list_page(&state, access.space(), &query, q.as_deref(), limit, offset).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/documents",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    request_body(content = DocumentInput, description = "An issued draft (`DocumentInput`), or with `direction: \"received\"` a received document (`ReceivedInput`)"),
    responses(
        (status = 201, body = Document),
        (status = 409, description = "`number_taken` (received: the allocated number is used)", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    access: Write,
    ApiJson(body): ApiJson<serde_json::Value>,
) -> Result<(StatusCode, Json<Document>), AppError> {
    let direction = body
        .get("direction")
        .and_then(|d| d.as_str())
        .map(str::trim);
    let id = if direction == Some(Direction::Received.as_str()) {
        received::create(&state, access.space(), body).await?
    } else {
        create_draft(&state, access.space(), from_value(body)?).await?
    };
    Ok((
        StatusCode::CREATED,
        Json(fetch(&state, access.space(), id).await?),
    ))
}

#[utoipa::path(
    get,
    path = "/api/documents/{id}",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Document), (status = 404, body = ErrorBody))
)]
pub async fn get(
    State(state): State<AppState>,
    access: Read,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Document>, AppError> {
    Ok(Json(fetch(&state, access.space(), id).await?))
}

#[utoipa::path(
    put,
    path = "/api/documents/{id}",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body(content = DocumentInput, description = "`DocumentInput` for an issued draft, `ReceivedInput` for a received document"),
    responses(
        (status = 200, body = Document),
        (status = 404, body = ErrorBody),
        (status = 409, description = "`document_locked` (an issued document that is not a draft)", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    access: Write,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(body): ApiJson<serde_json::Value>,
) -> Result<Json<Document>, AppError> {
    let space = access.space();
    let doc = query::find(&state.db, space, id).await?;
    if doc.direction == Direction::Received {
        received::update(&state, space, &doc, body).await?;
        return Ok(Json(fetch(&state, space, id).await?));
    }
    update_draft(&state, space, &doc, from_value(body)?).await?;
    Ok(Json(fetch(&state, space, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/documents/{id}",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204),
        (status = 404, body = ErrorBody),
        (status = 409, description = "`document_locked` (an issued document that is not a draft; received documents can always be deleted)", body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    access: Write,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    let space = access.space();
    let doc = query::find(&state.db, space, id).await?;
    let original = if doc.direction == Direction::Received {
        received_repo::delete(&state.db, space, id).await?
    } else {
        write::delete(&state.db, space, id).await?
    };
    // After the commit: a failed delete never loses the file.
    if let Some(rel) = original {
        state.pdf.space(space)?.storage().remove(&rel).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/documents/compute",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    request_body = ComputeInput,
    responses(
        (status = 200, body = Computed),
        (status = 422, description = "Same rules as save", body = ErrorBody),
    )
)]
pub async fn compute(
    State(state): State<AppState>,
    access: Write,
    ApiJson(input): ApiJson<ComputeInput>,
) -> Result<Json<Computed>, AppError> {
    Ok(Json(compute_totals(&state, access.space(), input).await?))
}
