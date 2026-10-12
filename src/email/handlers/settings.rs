//! `/api/settings/email`: configuration status, test message, templates.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{mailer, reply_to, sample_contexts};
use crate::app::AppState;
use crate::auth::ratelimit::TEMPLATE_SPACE;
use crate::auth::{Manage, Read, Write};
use crate::email::input::parse_address;
use crate::email::limits::{self, Kind};
use crate::email::sender::Outgoing;
use crate::email::templates::{self, Rendered, Template, TemplateInput};
use crate::error::{AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, optional_json};
use crate::pdf::format::Locale;
use crate::settings::repo::company;
use crate::space::SpaceId;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailStatus {
    /// SMTP is configured (`INVOICE__SMTP__HOST`).
    pub configured: bool,
    /// `INVOICE__EMAIL_DISABLED`: sending is switched off by the operator.
    pub disabled: bool,
    /// `INVOICE__SMTP__FROM` as configured.
    pub from: Option<String>,
    /// The company e-mail (`Reply-To`).
    pub reply_to: Option<String>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct TestInput {
    /// Default: the company e-mail.
    pub to: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateEntry {
    /// `cs` | `en`.
    pub locale: String,
    pub subject: String,
    pub body: String,
    /// An override is stored.
    pub custom: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateList {
    pub templates: Vec<TemplateEntry>,
}

fn entry(locale: Locale, t: Template, custom: bool) -> TemplateEntry {
    TemplateEntry {
        locale: locale.as_str().into(),
        subject: t.subject,
        body: t.body,
        custom,
    }
}

fn path_locale(s: &str) -> Result<Locale, AppError> {
    Locale::parse(s).ok_or(AppError::NotFound)
}

/// Compile + strict render on every sample (presence and limits are
/// checked by [`TemplateInput::validate`]); `TEMPLATE_SPACE` per space.
async fn validate(
    state: &AppState,
    space: SpaceId,
    locale: Locale,
    t: &Template,
) -> Result<Rendered, AppError> {
    state
        .limiter
        .reserve(&[(TEMPLATE_SPACE, &space.to_string())])?;
    let company = company::get(&state.db, space).await?;
    let samples = sample_contexts(&state.db, &company, locale).await?;
    state.templates.validate(t, &samples).await
}

#[utoipa::path(
    get,
    path = "/api/settings/email",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, body = EmailStatus))
)]
pub async fn status(
    State(state): State<AppState>,
    access: Read,
) -> Result<Json<EmailStatus>, AppError> {
    let company = company::get(&state.db, access.space()).await?;
    Ok(Json(EmailStatus {
        configured: state.email.is_some(),
        disabled: state.email_disabled,
        from: state.email.as_ref().map(|m| m.from().to_string()),
        reply_to: company.email.filter(|e| !e.trim().is_empty()),
    }))
}

#[utoipa::path(
    post,
    path = "/api/settings/email/test",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    request_body = TestInput,
    responses(
        (status = 204, description = "Sent"),
        (status = 422, description = "`to`: `required` (no address and no company e-mail) / `invalid`", body = ErrorBody),
        (status = 429, description = "`rate_limited` + `Retry-After`: the space's or the user's e-mails per hour / day are used up", body = ErrorBody),
        (status = 502, description = "`smtp_failed` (+ `detail`)", body = ErrorBody),
        (status = 503, description = "`email_disabled`, `smtp_not_configured`", body = ErrorBody),
    )
)]
pub async fn test(
    State(state): State<AppState>,
    access: Manage,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let input: TestInput = optional_json(&body)?;
    let mailer = mailer(&state)?;
    let company = company::get(&state.db, access.space()).await?;
    let to = input
        .to
        .filter(|t| !t.trim().is_empty())
        .or_else(|| company.email.clone().filter(|e| !e.trim().is_empty()))
        .ok_or(AppError::field("to", "required"))?;
    let to = parse_address(&to).ok_or(AppError::field("to", "invalid"))?;
    let (subject, text) = match Locale::parse(&company.default_locale) {
        Some(Locale::En) => (
            "Test e-mail",
            "This is a test e-mail from Invoice. The SMTP settings work.",
        ),
        _ => (
            "Testovací e-mail",
            "Toto je testovací e-mail z aplikace Invoice. Nastavení SMTP funguje.",
        ),
    };
    let msg = Outgoing {
        reply_to: reply_to(&company),
        to: vec![to],
        cc: vec![],
        bcc: vec![],
        subject: subject.into(),
        body: text.into(),
        files: vec![],
    };
    let s = access.scope;
    limits::reserve(&state.db, s.space, s.user_id, Kind::Test).await?;
    mailer.send(msg, &mailer.message_id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/settings/email/templates",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, body = TemplateList),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn list_templates(
    State(state): State<AppState>,
    access: Read,
) -> Result<Json<TemplateList>, AppError> {
    let pdf = state.pdf.space(access.space())?;
    let mut out = Vec::new();
    for locale in [Locale::Cs, Locale::En] {
        let (t, custom) = templates::load(pdf.storage(), locale).await?;
        out.push(entry(locale, t, custom));
    }
    Ok(Json(TemplateList { templates: out }))
}

#[utoipa::path(
    put,
    path = "/api/settings/email/templates/{locale}",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    params(("locale" = String, Path, description = "`cs` | `en`")),
    request_body = TemplateInput,
    responses(
        (status = 200, body = TemplateEntry),
        (status = 404, description = "Unknown locale", body = ErrorBody),
        (status = 422, description = "`validation` (`subject`: `required` / `too_long`, `body`: `required` / `too_long`) or `template_invalid` (+ `fields`, `detail`)", body = ErrorBody),
        (status = 429, description = "`rate_limited` + `Retry-After`: 60 template saves / previews per hour per space", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`, `busy` (every template renderer taken)", body = ErrorBody),
    )
)]
pub async fn put_template(
    State(state): State<AppState>,
    access: Manage,
    ApiPath(locale): ApiPath<String>,
    ApiJson(input): ApiJson<TemplateInput>,
) -> Result<Json<TemplateEntry>, AppError> {
    let locale = path_locale(&locale)?;
    let t = input.validate()?;
    validate(&state, access.space(), locale, &t).await?;
    let pdf = state.pdf.space(access.space())?;
    templates::save(pdf.storage(), locale, &t).await?;
    Ok(Json(entry(locale, t, true)))
}

#[utoipa::path(
    delete,
    path = "/api/settings/email/templates/{locale}",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    params(("locale" = String, Path, description = "`cs` | `en`")),
    responses(
        (status = 200, description = "The default template", body = TemplateEntry),
        (status = 404, description = "Unknown locale", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn delete_template(
    State(state): State<AppState>,
    access: Manage,
    ApiPath(locale): ApiPath<String>,
) -> Result<Json<TemplateEntry>, AppError> {
    let locale = path_locale(&locale)?;
    let pdf = state.pdf.space(access.space())?;
    templates::remove(pdf.storage(), locale).await?;
    Ok(Json(entry(locale, templates::default(locale), false)))
}

#[utoipa::path(
    post,
    path = "/api/settings/email/templates/{locale}/preview",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    params(("locale" = String, Path, description = "`cs` | `en`")),
    request_body = TemplateInput,
    responses(
        (status = 200, description = "Rendered on the sample invoice; nothing stored", body = Rendered),
        (status = 404, description = "Unknown locale", body = ErrorBody),
        (status = 422, description = "As for `PUT`", body = ErrorBody),
        (status = 429, description = "As for `PUT`", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`, `busy`", body = ErrorBody),
    )
)]
pub async fn preview(
    State(state): State<AppState>,
    access: Write,
    ApiPath(locale): ApiPath<String>,
    ApiJson(input): ApiJson<TemplateInput>,
) -> Result<Json<Rendered>, AppError> {
    let locale = path_locale(&locale)?;
    let t = input.validate()?;
    Ok(Json(validate(&state, access.space(), locale, &t).await?))
}
