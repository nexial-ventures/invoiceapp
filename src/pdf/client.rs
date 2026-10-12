//! mdcast client errors → API errors.

use mdcast_client::Error as E;
use mdcast_client::mdcast_api::wire::ErrorCode;

use crate::error::AppError;

/// Unreachable, credentials refused, or a gateway answering for a dead
/// upstream → 503 `pdf_unavailable`; a template that does not compile →
/// 502 `pdf_render_failed` with mdcast's message as `detail` (mdcast
/// sanitises it to `render failed (incident <id>)`; the typst diagnostics
/// are only in its logs); anything else → 502 without detail (logged only).
pub fn map_error(e: E) -> AppError {
    match e {
        E::Transport(_) | E::Unauthorized { .. } | E::Config(_) => {
            AppError::PdfUnavailable(e.to_string())
        }
        E::UnexpectedStatus { status, .. } | E::Server { status, .. }
            if (502..=504).contains(&status) =>
        {
            AppError::PdfUnavailable(e.to_string())
        }
        E::Server {
            code: ErrorCode::RenderFailed,
            message,
            ..
        } => AppError::PdfRenderFailed(message),
        other => AppError::PdfUpstream(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_errors_are_503() {
        for e in [
            E::Unauthorized {
                message: "bad token".into(),
            },
            E::Config("no scheme".into()),
            E::UnexpectedStatus {
                status: 502,
                body: "<html>".into(),
            },
            E::UnexpectedStatus {
                status: 504,
                body: "timeout".into(),
            },
            E::Server {
                status: 503,
                code: ErrorCode::Internal,
                message: "draining".into(),
            },
        ] {
            assert!(
                matches!(map_error(e), AppError::PdfUnavailable(_)),
                "should be unavailable"
            );
        }
    }

    #[test]
    fn render_failures_keep_the_message() {
        let e = E::Server {
            status: 422,
            code: ErrorCode::RenderFailed,
            message: "error: unknown variable: foo".into(),
        };
        assert!(matches!(
            map_error(e),
            AppError::PdfRenderFailed(m) if m == "error: unknown variable: foo"
        ));
    }

    #[test]
    fn other_failures_carry_no_detail() {
        for e in [
            E::PayloadTooLarge {
                message: "too big".into(),
            },
            E::UnexpectedStatus {
                status: 500,
                body: "<html>oops</html>".into(),
            },
            E::Server {
                status: 500,
                code: ErrorCode::Internal,
                message: "panic at server.rs:12".into(),
            },
            E::InvalidResponse {
                status: 409,
                reason: "expected value".into(),
            },
        ] {
            assert!(matches!(map_error(e), AppError::PdfUpstream(_)));
        }
    }
}
