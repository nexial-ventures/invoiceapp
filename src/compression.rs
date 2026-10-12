//! Response compression (brotli, gzip) for JSON, text, XML and the SPA's
//! JS / CSS / HTML / SVG. Already-compressed or binary bodies (PDF, zip,
//! images, fonts, octet streams) and SSE (the MCP stream) pass through
//! untouched, so their `Content-Length` and streaming stay as they are.

use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};

/// [`DefaultPredicate`] (skips tiny bodies, images but SVG, gRPC, SSE) and
/// the binary types (prefixes; `application/vnd.` covers the zip-based
/// office formats).
pub fn predicate() -> impl Predicate {
    DefaultPredicate::new()
        .and(NotForContentType::const_new("application/pdf"))
        .and(NotForContentType::const_new("application/zip"))
        .and(NotForContentType::const_new("application/gzip"))
        .and(NotForContentType::const_new("application/octet-stream"))
        .and(NotForContentType::const_new("application/vnd."))
        .and(NotForContentType::const_new("font/"))
}

pub fn layer() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new()
        .br(true)
        .gzip(true)
        .compress_when(predicate())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Response, header};

    fn compressed(content_type: &str) -> bool {
        let resp = Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CONTENT_LENGTH, "4096")
            .body(Body::from(vec![b'a'; 4096]))
            .expect("response");
        predicate().should_compress(&resp)
    }

    #[test]
    fn text_types_are_compressed() {
        for ct in [
            "application/json",
            "text/html; charset=utf-8",
            "text/javascript",
            "text/css",
            "text/csv; charset=utf-8",
            "application/xml",
            "image/svg+xml",
        ] {
            assert!(compressed(ct), "{ct}");
        }
    }

    #[test]
    fn compressed_and_binary_types_are_not() {
        for ct in [
            "application/pdf",
            "application/zip",
            "application/octet-stream",
            "image/png",
            "font/woff2",
            "text/event-stream",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ] {
            assert!(!compressed(ct), "{ct}");
        }
    }
}
