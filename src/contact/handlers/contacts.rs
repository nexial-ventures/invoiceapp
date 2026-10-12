use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use super::dto::{Contact, ContactInput, ContactList};
use crate::app::AppState;
use crate::auth::{Read, Write};
use crate::contact::repo::contacts as repo;
use crate::error::{AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::paging::ListQuery;

#[utoipa::path(
    get,
    path = "/api/contacts",
    tag = "contacts",
    security(("cookie" = []), ("bearer" = [])),
    params(ListQuery),
    responses((status = 200, body = ContactList))
)]
pub async fn list(
    State(state): State<AppState>,
    access: Read,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<ContactList>, AppError> {
    let (q, limit, offset) = query.normalized();
    let (items, total) = repo::list(&state.db, access.space(), q.as_deref(), limit, offset).await?;
    Ok(Json(ContactList {
        items: items.into_iter().map(Into::into).collect(),
        total,
    }))
}

#[utoipa::path(
    post,
    path = "/api/contacts",
    tag = "contacts",
    security(("cookie" = []), ("bearer" = [])),
    request_body = ContactInput,
    responses(
        (status = 201, body = Contact),
        (status = 422, description = "Validation failed / duplicate IČO", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    access: Write,
    ApiJson(input): ApiJson<ContactInput>,
) -> Result<(StatusCode, Json<Contact>), AppError> {
    let row = repo::create(&state.db, access.space(), input.validate()?).await?;
    Ok((StatusCode::CREATED, Json(row.into())))
}

#[utoipa::path(
    get,
    path = "/api/contacts/{id}",
    tag = "contacts",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Contact), (status = 404, body = ErrorBody))
)]
pub async fn get(
    State(state): State<AppState>,
    access: Read,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Contact>, AppError> {
    Ok(Json(repo::get(&state.db, access.space(), id).await?.into()))
}

#[utoipa::path(
    put,
    path = "/api/contacts/{id}",
    tag = "contacts",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ContactInput,
    responses(
        (status = 200, body = Contact),
        (status = 404, body = ErrorBody),
        (status = 422, description = "Validation failed / duplicate IČO", body = ErrorBody),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    access: Write,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ContactInput>,
) -> Result<Json<Contact>, AppError> {
    Ok(Json(
        repo::update(&state.db, access.space(), id, input.validate()?)
            .await?
            .into(),
    ))
}

#[utoipa::path(
    delete,
    path = "/api/contacts/{id}",
    tag = "contacts",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ErrorBody))
)]
pub async fn delete(
    State(state): State<AppState>,
    access: Write,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    repo::delete(&state.db, access.space(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}
