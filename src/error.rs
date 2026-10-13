//! Application error → client JSON `{ "code": ... }`.
//!
//! Internal and database errors are logged server-side and surface to the
//! client only as the generic `internal` code — never the underlying message.

use axum::http::StatusCode;
use sea_orm::DbErr;

#[path = "error_convert.rs"]
mod convert;
#[path = "error_fields.rs"]
mod fields;
#[path = "error_response.rs"]
mod response;
pub use convert::{lock_not_available, number_violation, unique_violation};
pub use fields::FieldErrors;
pub use response::ErrorBody;

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

    /// `POST /api/spaces` while `INVOICEAPP__REGISTRATION` is off: only invited
    /// people may enter the instance, so nobody creates a space of their own.
    #[error("space creation disabled")]
    SpaceCreationDisabled,

    /// `POST /api/spaces` by a user who already owns
    /// `INVOICEAPP__MAX_SPACES_PER_USER` spaces they created.
    #[error("space limit reached")]
    SpaceLimit,

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

    /// An action that needs TOTP (new recovery codes) while it is off.
    #[error("two-factor authentication not enabled")]
    MfaDisabled,

    /// An account for the address was registered meanwhile (invitation
    /// accept creating a new account).
    #[error("e-mail already registered")]
    EmailTaken,

    /// A cookie-authenticated mutation without the request's own `Origin`.
    #[error("CSRF origin check failed")]
    Csrf,

    /// Seconds until the limit frees up (`Retry-After`).
    #[error("rate limited")]
    RateLimited(u64),

    /// The password hashing capacity stayed saturated (argon2 gate, 503).
    #[error("busy")]
    Busy,

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

    /// Other documents refer to it (corrections, a settled final invoice…)
    /// and the delete / change would break them.
    #[error("the document is referenced by other documents")]
    RelatedInUse,

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

    /// An imported document that is already stored.
    #[error("document already imported")]
    Duplicate,

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

    /// Every render slot stayed taken (`pdf::limits`): `detail: "busy"`.
    #[error("PDF render queue full")]
    PdfBusy,

    /// mdcast answered `render_failed`; its message is returned as `detail`.
    /// mdcast sanitises it to `render failed (incident <id>)` — the typst
    /// diagnostics stay in mdcast's logs, the incident id finds them.
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

    /// No SMTP server configured (`INVOICEAPP__SMTP__HOST` unset).
    #[error("e-mail is not configured")]
    SmtpNotConfigured,

    /// `INVOICEAPP__EMAIL_DISABLED`: the operator's kill switch.
    #[error("e-mail is disabled")]
    EmailDisabled,

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

    /// A storage quota: the space's (`detail`: `<used> / <limit> MB`) or
    /// the design's own (`<used> / 50 MB`).
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
            Self::SpaceCreationDisabled => (StatusCode::FORBIDDEN, "space_creation_disabled"),
            Self::SpaceLimit => (StatusCode::FORBIDDEN, "space_limit"),
            Self::MfaRequired { .. } => (StatusCode::FORBIDDEN, "mfa_required"),
            Self::MfaInvalid => (StatusCode::UNAUTHORIZED, "mfa_invalid"),
            Self::MfaEnabled => (StatusCode::CONFLICT, "mfa_enabled"),
            Self::MfaDisabled => (StatusCode::CONFLICT, "mfa_disabled"),
            Self::EmailTaken => (StatusCode::CONFLICT, "email_taken"),
            Self::Csrf => (StatusCode::FORBIDDEN, "csrf"),
            Self::RateLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
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
            Self::RelatedInUse => (StatusCode::CONFLICT, "related_in_use"),
            Self::CatalogItemInUse => (StatusCode::CONFLICT, "catalog_item_in_use"),
            Self::CategoryInUse => (StatusCode::CONFLICT, "category_in_use"),
            Self::LastOwner => (StatusCode::CONFLICT, "last_owner"),
            Self::NumberTaken => (StatusCode::CONFLICT, "number_taken"),
            Self::Duplicate => (StatusCode::CONFLICT, "duplicate"),
            Self::ExceedsOriginal => (StatusCode::CONFLICT, "exceeds_original"),
            Self::PdfMissing => (StatusCode::NOT_FOUND, "pdf_missing"),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
            Self::PdfUnavailable(_) | Self::PdfBusy => {
                (StatusCode::SERVICE_UNAVAILABLE, "pdf_unavailable")
            }
            Self::StorageUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable"),
            Self::PdfRenderFailed(_) | Self::PdfUpstream(_) => {
                (StatusCode::BAD_GATEWAY, "pdf_render_failed")
            }
            Self::SmtpNotConfigured => (StatusCode::SERVICE_UNAVAILABLE, "smtp_not_configured"),
            Self::EmailDisabled => (StatusCode::SERVICE_UNAVAILABLE, "email_disabled"),
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

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
