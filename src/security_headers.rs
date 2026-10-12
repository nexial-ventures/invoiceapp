//! Security headers on every response, SPA and API alike (issue #40,
//! `docs/architecture.md`). A header a handler already set wins (the Google
//! redirects keep `Referrer-Policy: no-referrer`).

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::app::AppState;

/// Fits the built SPA: no inline script or style in `index.html` or the
/// bundle (Vue's `:style` bindings go through the CSSOM, which `style-src`
/// does not cover), vue-i18n compiles messages without `eval`, the payment
/// QR code is a `data:` SVG image, PDFs open as `blob:` URLs. `object-src
/// 'none'` does not stop the browsers' own PDF viewers (checked: Chromium
/// for inline responses and `blob:` tabs, Firefox for inline responses); no
/// `sandbox`, which would.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; object-src 'none'; \
     frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

pub const HSTS: &str = "max-age=31536000; includeSubDomains";

pub async fn security_headers(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    apply(resp.headers_mut(), state.public.https);
    resp
}

fn apply(h: &mut HeaderMap, https: bool) {
    let mut set = |name: HeaderName, value: &'static str| {
        h.entry(name)
            .or_insert_with(|| HeaderValue::from_static(value));
    };
    set(header::CONTENT_SECURITY_POLICY, CSP);
    set(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(header::REFERRER_POLICY, "strict-origin-when-cross-origin");
    set(header::X_FRAME_OPTIONS, "DENY");
    // Only over TLS: on plain http the header is ignored at best, and a
    // dev instance must not pin `localhost` subdomains to https.
    if https {
        set(header::STRICT_TRANSPORT_SECURITY, HSTS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsts_only_over_https() {
        let mut h = HeaderMap::new();
        apply(&mut h, false);
        assert!(!h.contains_key(header::STRICT_TRANSPORT_SECURITY));
        assert_eq!(h[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let mut h = HeaderMap::new();
        apply(&mut h, true);
        assert_eq!(h[header::STRICT_TRANSPORT_SECURITY], HSTS);
    }

    #[test]
    fn a_handler_set_header_wins() {
        let mut h = HeaderMap::new();
        h.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        apply(&mut h, true);
        assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(h.get_all(header::REFERRER_POLICY).iter().count(), 1);
    }

    #[test]
    fn the_csp_allows_no_inline_code() {
        for d in CSP.split(';').map(str::trim) {
            assert!(!d.contains("unsafe"), "{d}");
        }
        assert!(CSP.contains("object-src 'none'") && !CSP.contains("sandbox"));
    }
}
