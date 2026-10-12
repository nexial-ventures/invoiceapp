//! Embedded Vue SPA (`frontend/dist`) served for every non-`/api` path.

use std::borrow::Cow;

use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use rust_embed::Embed;

// `allow_missing` lets the crate build (and clippy/tests run) before the
// frontend has ever been built; the handler then answers 503.
#[derive(Embed)]
#[folder = "frontend/dist"]
#[allow_missing = true]
struct FrontendAssets;

pub async fn serve(req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');

    if let Some(asset) = FrontendAssets::get(path) {
        return asset_response(path, asset.data);
    }

    // A missing file with an extension must 404 — falling through to
    // index.html would make the browser parse HTML as JS/CSS.
    if has_extension(path) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    // Client-side route: hand index.html to the Vue router.
    match FrontendAssets::get("index.html") {
        Some(asset) => asset_response("index.html", asset.data),
        None => (StatusCode::SERVICE_UNAVAILABLE, "frontend not built").into_response(),
    }
}

fn has_extension(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|s| s.contains('.'))
}

/// Vite content-hashes everything under `assets/`, so those URLs never change
/// content. Everything else (index.html above all) must revalidate, or a
/// stale index.html keeps pointing at the previous build's bundles.
fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

/// Release builds embed the files (`Cow::Borrowed` of static bytes, served
/// without a copy); debug builds read them from disk (`Cow::Owned`).
fn asset_response(path: &str, data: Cow<'static, [u8]>) -> Response {
    let body = match data {
        Cow::Borrowed(b) => Bytes::from_static(b),
        Cow::Owned(v) => Bytes::from(v),
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache_control(path)),
        ],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashed_assets_are_immutable() {
        assert_eq!(
            cache_control("assets/index-abc123.js"),
            "public, max-age=31536000, immutable"
        );
    }

    #[test]
    fn everything_else_revalidates() {
        assert_eq!(cache_control("index.html"), "no-cache");
        assert_eq!(cache_control("favicon.svg"), "no-cache");
    }

    #[test]
    fn detects_file_extensions() {
        assert!(has_extension("assets/app.js"));
        assert!(has_extension("favicon.ico"));
        assert!(!has_extension("invoices/42"));
        assert!(!has_extension(""));
        assert!(!has_extension("v1.0/settings"));
    }

    #[tokio::test]
    async fn missing_asset_with_extension_is_404() {
        let req = Request::builder()
            .uri("/assets/definitely-missing-xyz.js")
            .body(axum::body::Body::empty())
            .expect("request");
        assert_eq!(serve(req).await.status(), StatusCode::NOT_FOUND);
    }
}
