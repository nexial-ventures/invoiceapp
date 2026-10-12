//! MCP endpoint (`POST /api/mcp`): a stateless Streamable HTTP server
//! (`rmcp`) whose tools are a thin layer over the REST handlers' service
//! functions. Contract: `docs/api/mcp.md`.

mod args;
mod params;
mod read;
mod result;
mod write;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, tool_handler};
use serde_json::Value;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::Authed;
use crate::document::service::fetch;
use crate::error::{AppError, ErrorBody};
use crate::space::{Role, SpaceId};
use axum::http::request::Parts;
use result::{pdf_url, to_value, with};

/// Same as axum's default JSON body limit.
pub const BODY_LIMIT: usize = 2 * 1024 * 1024;

const INSTRUCTIONS: &str = "\
Invoice management of one Czech company (this space; what you may do depends on your token's role).
Documents have a direction: `issued` (our invoices to customers) or `received` (supplier invoices, read only here). \
Issued types: `invoice`, `proforma` (advance request, not a tax document), `simplified` (simplified tax document); \
also `advance_tax_doc` (DDPP, created automatically when a VAT payer's proforma is paid), `credit_note`, `debit_note`.
Status: `draft` (editable, no number) -> `issued` (numbered, immutable, PDF archived) -> `cancelled`. \
paymentState (issued only): unpaid | partial | paid | overpaid; overdue = unpaid / partial past dueDate.
Money, quantities and rates are decimal strings (\"1210.00\", \"21\"); dates are YYYY-MM-DD. \
VAT: vatMode standard | reverse_charge | exempt | non_payer; line vatRate is a percent.
To issue an invoice: find the customer with list_contacts (or lookup_ares by IČO, then create_contact) -> \
create_draft with contactId and item lines -> check the returned totals (compute_document previews without saving) -> \
issue_document (irreversible: assigns the number and renders the PDF). Then add_payment / mark_sent as they happen.
Validation errors come back as tool errors {\"code\":\"validation\",\"fields\":{\"<field>\":\"<reason>\"}}. \
PDFs are not returned here: download pdfUrl (relative to this server) with the same Bearer token.";

#[derive(Clone)]
pub struct InvoiceMcp {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

impl InvoiceMcp {
    pub fn tools() -> ToolRouter<Self> {
        Self::read_tools() + Self::write_tools()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for InvoiceMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("invoice", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Routes relative to `/api/mcp` (mounted behind the auth middleware).
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route_service("/", service(state))
        .layer(axum::middleware::map_response(json_errors))
}

/// The rmcp tower service. Stateless: no `Mcp-Session-Id`, every POST stands
/// alone, GET / DELETE → 405.
fn service(state: AppState) -> StreamableHttpService<InvoiceMcp, NeverSessionManager> {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        // Served on the space's own host; the auth middleware (token or
        // session), not a loopback Host allowlist, is what guards it.
        .disable_allowed_hosts()
        .with_max_request_body_bytes(BODY_LIMIT);
    let tool_router = InvoiceMcp::tools();
    StreamableHttpService::new(
        move || {
            Ok(InvoiceMcp {
                state: state.clone(),
                tool_router: tool_router.clone(),
            })
        },
        Default::default(),
        config,
    )
}

/// The `{"code"}` of rmcp's plain-text HTTP-level rejections.
fn http_error_code(status: StatusCode) -> Option<&'static str> {
    match status {
        StatusCode::METHOD_NOT_ALLOWED => Some("method_not_allowed"),
        StatusCode::PAYLOAD_TOO_LARGE => Some("too_large"),
        StatusCode::BAD_REQUEST
        | StatusCode::NOT_ACCEPTABLE
        | StatusCode::UNSUPPORTED_MEDIA_TYPE => Some("bad_request"),
        _ => None,
    }
}

/// rmcp answers transport errors with plain text; the API's error shape is
/// JSON `{"code"}`. JSON bodies (JSON-RPC errors) pass through untouched.
async fn json_errors(resp: Response) -> Response {
    let is_json = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    let Some(code) = http_error_code(resp.status()).filter(|_| !is_json) else {
        return resp;
    };
    tracing::debug!(status = %resp.status(), code, "MCP transport request rejected");
    let allow: Option<HeaderValue> = resp.headers().get(header::ALLOW).cloned();
    let body = ErrorBody {
        code,
        fields: None,
        detail: None,
    };
    let mut out = (resp.status(), axum::Json(body)).into_response();
    if let Some(allow) = allow {
        out.headers_mut().insert(header::ALLOW, allow);
    }
    out
}

/// Run a tool in the caller's space at `min` role or above; below it the
/// tool error `{"code":"forbidden"}`. The auth middleware in front of the
/// MCP service put the caller into the request extensions.
async fn in_space<T, Fut>(
    parts: &Parts,
    min: Role,
    f: impl FnOnce(SpaceId) -> Fut,
) -> Result<T, AppError>
where
    Fut: Future<Output = Result<T, AppError>>,
{
    let authed = parts
        .extensions
        .get::<Authed>()
        .ok_or(AppError::Unauthorized)?;
    f(authed.scope(min)?.space).await
}

/// A document as `GET /api/documents/{id}` returns it, plus `pdfUrl`.
async fn document_value(state: &AppState, space: SpaceId, id: Uuid) -> Result<Value, AppError> {
    let doc = fetch(state, space, id).await?;
    let url = pdf_url(doc.id, doc.direction, doc.imported, doc.original.is_some());
    Ok(with(to_value(&doc)?, "pdfUrl", url))
}

/// OpenAPI entry only: the route is served by [`service`].
#[utoipa::path(
    post,
    path = "/api/mcp",
    tag = "mcp",
    security(("cookie" = []), ("bearer" = [])),
    request_body(
        content = Object,
        description = "One JSON-RPC 2.0 message (MCP Streamable HTTP, stateless): `initialize`, `tools/list`, `tools/call`, … See docs/api/mcp.md."
    ),
    responses(
        (status = 200, description = "JSON-RPC response (`application/json`)", body = Object),
        (status = 202, description = "Notification accepted"),
        (status = 401, description = "Missing / wrong Bearer token", body = ErrorBody),
        (status = 405, description = "GET / DELETE (stateless server, no session or SSE stream)"),
        (status = 406, description = "`Accept` must list both `application/json` and `text/event-stream`"),
    )
)]
#[allow(dead_code)]
pub(crate) fn openapi_mcp() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_errors_get_codes() {
        let code = |s| http_error_code(s);
        assert_eq!(
            code(StatusCode::METHOD_NOT_ALLOWED),
            Some("method_not_allowed")
        );
        assert_eq!(code(StatusCode::PAYLOAD_TOO_LARGE), Some("too_large"));
        assert_eq!(code(StatusCode::NOT_ACCEPTABLE), Some("bad_request"));
        assert_eq!(
            code(StatusCode::UNSUPPORTED_MEDIA_TYPE),
            Some("bad_request")
        );
        assert_eq!(code(StatusCode::OK), None);
        assert_eq!(code(StatusCode::ACCEPTED), None);
    }
}
