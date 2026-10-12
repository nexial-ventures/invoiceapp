use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::{Manage, Read};
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::{ApiJson, ApiPath};
use crate::num_parse::parse_rate;
use crate::settings::entity::vat_rate;
use crate::settings::repo;
use crate::validation as v;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VatRate {
    pub id: Uuid,
    /// Decimal percentage as a string, e.g. `"21"`, `"12.5"`.
    pub rate: Decimal,
    pub label: String,
    pub is_default: bool,
    pub active: bool,
    pub position: i32,
}

impl From<vat_rate::Model> for VatRate {
    fn from(m: vat_rate::Model) -> Self {
        Self {
            id: m.id,
            rate: m.rate.normalize(),
            label: m.label,
            is_default: m.is_default,
            active: m.active,
            position: m.position,
        }
    }
}

/// Create/update body. `position` defaults to the end of the list on create
/// and to the current position on update.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VatRateInput {
    /// Decimal string 0..100 with at most 2 decimal places.
    #[serde(default)]
    pub rate: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default = "default_true")]
    pub active: bool,
    pub position: Option<i32>,
}

fn default_true() -> bool {
    true
}

/// Validated [`VatRateInput`].
#[derive(Debug, Clone, PartialEq)]
pub struct VatRateData {
    pub rate: Decimal,
    pub label: String,
    pub is_default: bool,
    pub active: bool,
    pub position: Option<i32>,
}

impl VatRateInput {
    pub fn validate(self) -> Result<VatRateData, AppError> {
        let mut e = FieldErrors::new();
        let out = VatRateData {
            rate: e.check("rate", parse_rate(&self.rate)).unwrap_or_default(),
            label: e
                .check("label", v::required_text(&self.label, 100))
                .unwrap_or_default(),
            is_default: self.is_default,
            active: self.active,
            position: self.position,
        };
        if self.is_default && !self.active {
            e.add("isDefault", "invalid");
        }
        e.into_result()?;
        Ok(out)
    }
}

#[utoipa::path(
    get,
    path = "/api/settings/vat-rates",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, body = Vec<VatRate>))
)]
pub async fn list(
    State(state): State<AppState>,
    access: Read,
) -> Result<Json<Vec<VatRate>>, AppError> {
    let rows = repo::vat_rates::list(&state.db, access.space()).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    post,
    path = "/api/settings/vat-rates",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    request_body = VatRateInput,
    responses(
        (status = 201, body = VatRate),
        (status = 422, description = "Validation failed / duplicate rate / inactive default", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    access: Manage,
    ApiJson(input): ApiJson<VatRateInput>,
) -> Result<(StatusCode, Json<VatRate>), AppError> {
    let row = repo::vat_rates::create(&state.db, access.space(), input.validate()?).await?;
    Ok((StatusCode::CREATED, Json(row.into())))
}

#[utoipa::path(
    put,
    path = "/api/settings/vat-rates/{id}",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = VatRateInput,
    responses(
        (status = 200, body = VatRate),
        (status = 404, body = ErrorBody),
        (status = 422, description = "Validation failed / duplicate rate / inactive default", body = ErrorBody),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    access: Manage,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<VatRateInput>,
) -> Result<Json<VatRate>, AppError> {
    let row = repo::vat_rates::update(&state.db, access.space(), id, input.validate()?).await?;
    Ok(Json(row.into()))
}

#[utoipa::path(
    delete,
    path = "/api/settings/vat-rates/{id}",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ErrorBody))
)]
pub async fn delete(
    State(state): State<AppState>,
    access: Manage,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    repo::vat_rates::delete(&state.db, access.space(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_must_be_active() {
        let input = |is_default, active| VatRateInput {
            rate: "5".into(),
            label: "X".into(),
            is_default,
            active,
            position: None,
        };
        assert!(input(true, true).validate().is_ok());
        assert!(input(false, false).validate().is_ok());
        let Err(AppError::Validation(fields)) = input(true, false).validate() else {
            panic!("expected validation error");
        };
        let mut expected = FieldErrors::new();
        expected.add("isDefault", "invalid");
        assert_eq!(fields, expected);
    }

    #[test]
    fn rate_serializes_as_normalized_string() {
        let dto = VatRate::from(vat_rate::Model {
            id: Uuid::nil(),
            space_id: Uuid::nil(),
            rate: Decimal::new(2100, 2),
            label: "Základní".into(),
            is_default: true,
            active: true,
            position: 1,
        });
        assert_eq!(serde_json::to_value(&dto).expect("json")["rate"], "21");
    }
}
