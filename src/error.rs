//! Application error → client JSON `{ "code": ... }`.
//!
//! Internal and database errors are logged server-side and surface to the
//! client only as the generic `internal` code — never the underlying message.

use std::collections::BTreeMap;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sea_orm::DbErr;
use serde::Serialize;
use utoipa::ToSchema;

#[path = "error_convert.rs"]
mod convert;
#[path = "error_fields.rs"]
mod fields;
pub use convert::{lock_not_available, number_violation, unique_violation};
pub use fields::FieldErrors;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized")]
    Unauthorized,

    /// Login refused (wrong e-mail / password, not a member, unverified on a
    /// space host, disabled) — one code for all, no account enumeration.
    #[error("invalid credentials")]
    InvalidCredentials,

    /// The caller's effective role is below the route's minimum.
    #[error("forbidden")]
    Forbidden,

    #[error("e-mail not verified")]
    EmailUnverified,

    /// The space requires TOTP and the user has none (login, invitation
    /// accept). `account_created`: an invitation created the account
    /// without a membership (`detail: "account_created"`).
    #[error("two-factor authentication required")]
    MfaRequired { account_created: bool },

    /// A wrong second-factor code at the login's code step.
    #[error("invalid two-factor code")]
    MfaInvalid,

    /// TOTP setup while it is already on.
    #[error("two-factor authentication already enabled")]
    MfaEnabled,

    /// A cookie-authenticated mutation without the request's own `Origin`.
    #[error("CSRF origin check failed")]
    Csrf,

    /// Seconds until the limit frees up (`Retry-After`).
    #[error("rate limited")]
    RateLimited(u64),

    #[error("not found")]
    NotFound,

    #[error("malformed request: {0}")]
    BadRequest(String),

    #[error("validation failed: {0:?}")]
    Validation(FieldErrors),

    /// A validation error whose `detail` names the offending part of the
    /// input (e.g. the CSV column a file error is about).
    #[error("validation failed: {0:?} ({1})")]
    ValidationDetail(FieldErrors, String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("ARES subject not found")]
    AresNotFound,

    #[error("ARES unavailable: {0:#}")]
    AresUnavailable(anyhow::Error),

    #[error("document is locked (not a draft)")]
    DocumentLocked,

    #[error("action not allowed in the current document state")]
    InvalidState,

    #[error("ČNB unavailable: {0:#}")]
    CnbUnavailable(anyhow::Error),

    #[error("the advance is deducted by an issued invoice")]
    AdvanceSettled,

    #[error("the advance is deducted by a draft invoice")]
    AdvanceInUse,

    #[error("the catalog item is the last member of a group")]
    CatalogItemInUse,

    #[error("the category is used by a document")]
    CategoryInUse,

    /// The change would leave the space without an owner.
    #[error("the last owner cannot leave")]
    LastOwner,

    /// The allocated number is already held by another (imported) document.
    #[error("document number already taken")]
    NumberTaken,

    /// Cancelling a debit note would leave its original over-credited.
    #[error("the credit notes would exceed the original")]
    ExceedsOriginal,

    #[error("the document has no PDF")]
    PdfMissing,

    #[error("upload too large")]
    TooLarge,

    /// mdcast unreachable, token rejected, or a gateway answering for it.
    #[error("PDF service unavailable: {0}")]
    PdfUnavailable(String),

    /// mdcast answered but the render failed; the message (typst diagnostics
    /// of the user's own template) is returned to the client as `detail`.
    #[error("PDF render failed: {0}")]
    PdfRenderFailed(String),

    /// mdcast failed in a way that is not the template's fault (its own
    /// 500, an undecodable answer…): 502 `pdf_render_failed` without
    /// `detail` — the upstream body is logged, never returned.
    #[error("PDF service error: {0}")]
    PdfUpstream(String),

    /// The file storage (directory or S3 bucket) failed; the cause is logged.
    #[error("storage unavailable: {0}")]
    StorageUnavailable(String),

    /// No SMTP server configured (`INVOICE__SMTP__HOST` unset).
    #[error("e-mail is not configured")]
    SmtpNotConfigured,

    /// The SMTP server refused the message or could not be reached; the
    /// server's response / transport error (never credentials) is returned
    /// as `detail`.
    #[error("SMTP failed: {0}")]
    SmtpFailed(String),

    /// An e-mail template does not compile or render on the sample:
    /// `fields.{field}` = `template_invalid`, `detail` = line + message.
    #[error("template {field} invalid: {detail}")]
    TemplateInvalid { field: &'static str, detail: String },

    /// The design does not render the sample (`detail`: `<locale>: <message>`).
    #[error("design invalid: {0}")]
    DesignInvalid(String),

    #[error("the design draft is empty")]
    DraftEmpty,

    /// The space's design storage quota (`detail`: `<used> / 50 MB`).
    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),

    /// Another design change of the space holds its lock.
    #[error("the design is being changed")]
    DesignBusy,

    #[error("the draft changed during the check")]
    DraftChanged,

    /// A version's stored files are missing or differ from its record.
    #[error("the design version is incomplete")]
    VersionIncomplete,

    #[error("internal error: {0:#}")]
    Internal(#[from] anyhow::Error),

    #[error("database error: {0}")]
    Database(DbErr),
}

impl AppError {
    /// Single-field validation error.
    pub fn field(field: &str, reason: &'static str) -> Self {
        let mut errors = FieldErrors::new();
        errors.add(field, reason);
        Self::Validation(errors)
    }

    /// Single-field validation error with a `detail`.
    pub fn field_detail(field: &str, reason: &'static str, detail: impl Into<String>) -> Self {
        let mut errors = FieldErrors::new();
        errors.add(field, reason);
        Self::ValidationDetail(errors, detail.into())
    }

    pub fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::EmailUnverified => (StatusCode::FORBIDDEN, "email_unverified"),
            Self::MfaRequired { .. } => (StatusCode::FORBIDDEN, "mfa_required"),
            Self::MfaInvalid => (StatusCode::UNAUTHORIZED, "mfa_invalid"),
            Self::MfaEnabled => (StatusCode::CONFLICT, "mfa_enabled"),
            Self::Csrf => (StatusCode::FORBIDDEN, "csrf"),
            Self::RateLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Validation(_) | Self::ValidationDetail(..) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "validation")
            }
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::AresNotFound => (StatusCode::NOT_FOUND, "ares_not_found"),
            Self::AresUnavailable(_) => (StatusCode::BAD_GATEWAY, "ares_unavailable"),
            Self::DocumentLocked => (StatusCode::CONFLICT, "document_locked"),
            Self::InvalidState => (StatusCode::CONFLICT, "invalid_state"),
            Self::CnbUnavailable(_) => (StatusCode::BAD_GATEWAY, "cnb_unavailable"),
            Self::AdvanceSettled => (StatusCode::CONFLICT, "advance_settled"),
            Self::AdvanceInUse => (StatusCode::CONFLICT, "advance_in_use"),
            Self::CatalogItemInUse => (StatusCode::CONFLICT, "catalog_item_in_use"),
            Self::CategoryInUse => (StatusCode::CONFLICT, "category_in_use"),
            Self::LastOwner => (StatusCode::CONFLICT, "last_owner"),
            Self::NumberTaken => (StatusCode::CONFLICT, "number_taken"),
            Self::ExceedsOriginal => (StatusCode::CONFLICT, "exceeds_original"),
            Self::PdfMissing => (StatusCode::NOT_FOUND, "pdf_missing"),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
            Self::PdfUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "pdf_unavailable"),
            Self::StorageUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable"),
            Self::PdfRenderFailed(_) | Self::PdfUpstream(_) => {
                (StatusCode::BAD_GATEWAY, "pdf_render_failed")
            }
            Self::SmtpNotConfigured => (StatusCode::SERVICE_UNAVAILABLE, "smtp_not_configured"),
            Self::SmtpFailed(_) => (StatusCode::BAD_GATEWAY, "smtp_failed"),
            Self::TemplateInvalid { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "template_invalid"),
            Self::DesignInvalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "design_invalid"),
            Self::DraftEmpty => (StatusCode::UNPROCESSABLE_ENTITY, "draft_empty"),
            Self::QuotaExceeded(_) => (StatusCode::UNPROCESSABLE_ENTITY, "quota_exceeded"),
            Self::DesignBusy => (StatusCode::CONFLICT, "design_busy"),
            Self::DraftChanged => (StatusCode::CONFLICT, "draft_changed"),
            Self::VersionIncomplete => (StatusCode::CONFLICT, "version_incomplete"),
            Self::Internal(_) | Self::Database(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        }
    }
}

/// Error body returned by every failing API call.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// Machine-readable error code, e.g. `unauthorized`, `not_found`, `validation`, `internal`.
    pub code: &'static str,
    /// Only for `validation` / `template_invalid`: camelCase field name → reason code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<BTreeMap<String, String>>,
    /// For `mfa_required` after an invitation created the account
    /// (`account_created`), `pdf_render_failed` (the render service's message), `smtp_failed`
    /// (the SMTP server's response), `template_invalid` (line + message),
    /// `design_invalid` (locale + render message), `quota_exceeded` (usage) and a
    /// `validation` error that names the offending input part (e.g. the CSV
    /// column of a `missing_column` file error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AppError {
    /// Log the cause server-side (internal / upstream detail never reaches the client).
    pub fn log(&self) {
        match self {
            Self::Internal(_) | Self::Database(_) => {
                tracing::error!(error = %self, "request failed")
            }
            Self::AresUnavailable(_) => tracing::warn!(error = %self, "ARES lookup failed"),
            Self::CnbUnavailable(_) => tracing::warn!(error = %self, "ČNB lookup failed"),
            Self::PdfUnavailable(_) | Self::PdfRenderFailed(_) | Self::PdfUpstream(_) => {
                tracing::warn!(error = %self, "PDF render failed")
            }
            Self::StorageUnavailable(_) => tracing::error!(error = %self, "storage failed"),
            Self::SmtpFailed(_) => tracing::warn!(error = %self, "e-mail not sent"),
            Self::BadRequest(_) | Self::Conflict(_) => tracing::debug!(error = %self, "rejected"),
            _ => {}
        }
    }

    /// The client-facing error body (REST response, MCP tool error).
    pub fn body(&self) -> ErrorBody {
        let (_, code) = self.status_and_code();
        let fields = match self {
            Self::Validation(f) | Self::ValidationDetail(f, _) => Some(
                f.0.iter()
                    .map(|(k, v)| (k.clone(), (*v).to_string()))
                    .collect(),
            ),
            Self::TemplateInvalid { field, .. } => Some(BTreeMap::from([(
                field.to_string(),
                "template_invalid".into(),
            )])),
            _ => None,
        };
        let detail = match self {
            Self::PdfRenderFailed(d)
            | Self::SmtpFailed(d)
            | Self::ValidationDetail(_, d)
            | Self::DesignInvalid(d)
            | Self::QuotaExceeded(d) => Some(d.clone()),
            Self::TemplateInvalid { detail, .. } => Some(detail.clone()),
            Self::MfaRequired {
                account_created: true,
            } => Some("account_created".into()),
            _ => None,
        };
        ErrorBody {
            code,
            fields,
            detail,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        self.log();
        let (status, _) = self.status_and_code();
        let mut resp = (status, Json(self.body())).into_response();
        match self {
            Self::Unauthorized => {
                resp.headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            }
            Self::RateLimited(secs) => {
                resp.headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from(secs));
            }
            _ => {}
        }
        resp
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
