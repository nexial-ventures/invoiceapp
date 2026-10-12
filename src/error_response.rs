//! The client-facing side of [`AppError`]: the JSON body, the log line and
//! the HTTP response.

use std::collections::BTreeMap;

use axum::Json;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

use super::AppError;

/// Error body returned by every failing API call.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// Machine-readable error code, e.g. `unauthorized`, `not_found`, `validation`, `internal`.
    pub code: &'static str,
    /// Only for `validation` / `template_invalid`: camelCase field name → reason code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<BTreeMap<String, String>>,
    /// For `mfa_required` after an invitation created the account
    /// (`account_created`), `pdf_render_failed` (the render service's message, an incident id),
    /// `pdf_unavailable` when every render slot is taken (`busy`),
    /// `smtp_failed`
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
            Self::PdfUnavailable(_)
            | Self::PdfBusy
            | Self::PdfRenderFailed(_)
            | Self::PdfUpstream(_) => {
                tracing::warn!(error = %self, "PDF render failed")
            }
            Self::StorageUnavailable(_) => tracing::error!(error = %self, "storage failed"),
            Self::Busy => tracing::warn!(
                "capacity saturated (password hashing or e-mail templates), request refused"
            ),
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
            Self::PdfBusy => Some("busy".into()),
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
            Self::PdfBusy => {
                resp.headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from_static("10"));
            }
            _ => {}
        }
        resp
    }
}
