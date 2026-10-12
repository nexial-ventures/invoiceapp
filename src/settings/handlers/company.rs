use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::auth::{Manage, Read};
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::ApiJson;
use crate::settings::entity::company;
use crate::settings::handlers::company_defaults::{DefaultsUpdate, DocDefaults, DocDefaultsInput};
use crate::settings::repo;
use crate::validation as v;

/// Own company profile (one per space) with its document defaults.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Company {
    #[serde(flatten)]
    pub profile: Profile,
    #[serde(flatten)]
    pub defaults: DocDefaults,
}

/// `PUT` body: the profile (replaced) and the document defaults (each
/// optional, absent = keep the stored value).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CompanyInput {
    #[serde(flatten)]
    pub profile: ProfileInput,
    #[serde(flatten)]
    pub defaults: DocDefaultsInput,
}

/// The profile fields of a `PUT`. `vatPayer`, `defaultDueDays` and
/// `defaultLocale` are required; they are optional here only so that a
/// missing one is a 422 `required` instead of a serde 400.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    #[serde(default)]
    pub name: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub vat_payer: Option<bool>,
    #[serde(default)]
    pub street: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub zip: String,
    #[serde(default)]
    pub country: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub web: Option<String>,
    pub registration: Option<String>,
    pub default_due_days: Option<i32>,
    /// `cs` or `en`.
    pub default_locale: Option<String>,
}

fn present<T>(value: Option<T>) -> v::Check<T> {
    value.ok_or("required")
}

/// The profile fields of a response (and the validated `PUT`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub name: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub vat_payer: bool,
    #[serde(default)]
    pub street: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub zip: String,
    #[serde(default)]
    pub country: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub web: Option<String>,
    pub registration: Option<String>,
    pub default_due_days: i32,
    /// `cs` or `en`.
    pub default_locale: String,
}

impl CompanyInput {
    /// Normalize every field or report all failures at once.
    pub fn validate(self) -> Result<(Profile, DefaultsUpdate), AppError> {
        let mut e = FieldErrors::new();
        let defaults = self.defaults.validate(&mut e);
        let p = self.profile;
        let out = Profile {
            name: e
                .check("name", v::required_text(&p.name, 200))
                .unwrap_or_default(),
            ico: e.check("ico", v::opt_ico(p.ico.as_deref())).flatten(),
            dic: e.check("dic", v::opt_dic(p.dic.as_deref())).flatten(),
            vat_payer: e
                .check("vatPayer", present(p.vat_payer))
                .unwrap_or_default(),
            street: e
                .check("street", v::text(&p.street, 200))
                .unwrap_or_default(),
            city: e.check("city", v::text(&p.city, 100)).unwrap_or_default(),
            zip: e.check("zip", v::text(&p.zip, 20)).unwrap_or_default(),
            country: e
                .check("country", v::country(&p.country))
                .unwrap_or_default(),
            email: e.check("email", v::opt_email(p.email.as_deref())).flatten(),
            phone: e
                .check("phone", v::opt_text(p.phone.as_deref(), 50))
                .flatten(),
            web: e.check("web", v::opt_text(p.web.as_deref(), 200)).flatten(),
            registration: e
                .check("registration", v::opt_text(p.registration.as_deref(), 300))
                .flatten(),
            default_due_days: e
                .check(
                    "defaultDueDays",
                    present(p.default_due_days).and_then(v::due_days),
                )
                .unwrap_or_default(),
            default_locale: e
                .check(
                    "defaultLocale",
                    present(p.default_locale.as_deref()).and_then(v::locale),
                )
                .unwrap_or_default(),
        };
        e.into_result()?;
        Ok((out, defaults))
    }
}

impl TryFrom<company::Model> for Company {
    type Error = anyhow::Error;

    fn try_from(m: company::Model) -> anyhow::Result<Self> {
        Ok(Self {
            defaults: DocDefaults::try_from(&m)?,
            profile: m.into(),
        })
    }
}

impl From<company::Model> for Profile {
    fn from(m: company::Model) -> Self {
        Self {
            name: m.name,
            ico: m.ico,
            dic: m.dic,
            vat_payer: m.vat_payer,
            street: m.street,
            city: m.city,
            zip: m.zip,
            country: m.country,
            email: m.email,
            phone: m.phone,
            web: m.web,
            registration: m.registration,
            default_due_days: m.default_due_days,
            default_locale: m.default_locale,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/settings/company",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, body = Company))
)]
pub async fn get_company(
    State(state): State<AppState>,
    access: Read,
) -> Result<Json<Company>, AppError> {
    let row = repo::company::get(&state.db, access.space()).await?;
    Ok(Json(Company::try_from(row)?))
}

#[utoipa::path(
    put,
    path = "/api/settings/company",
    tag = "settings",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CompanyInput,
    responses(
        (status = 200, body = Company),
        (status = 422, description = "Validation failed", body = ErrorBody),
    )
)]
pub async fn put_company(
    State(state): State<AppState>,
    access: Manage,
    ApiJson(input): ApiJson<CompanyInput>,
) -> Result<Json<Company>, AppError> {
    let (profile, defaults) = input.validate()?;
    let row = repo::company::update(&state.db, access.space(), profile, defaults).await?;
    Ok(Json(Company::try_from(row)?))
}
