//! Sign in with Google (google.md, 4d) through the HTTP routes against the
//! mock provider: start → (the test plays Google) → callback → finish.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode, header};
use sea_orm::DatabaseConnection;
use serde_json::Value;

use super::BASE_HOST;
use super::auth::{exchange, request, session_cookie};
use super::mfa::pending_cookie;
use super::oidc::{self, CLIENT_ID, OidcMock};

/// The app with Google pointed at a fresh mock provider.
pub fn app(db: &DatabaseConnection, registration: bool) -> (Router, OidcMock) {
    app_with(
        db,
        registration,
        invoiceapp::auth::google::provider::JWKS_REFRESH,
    )
}

/// [`app`] with another minimum interval between JWKS refetches.
pub fn app_with(
    db: &DatabaseConnection,
    registration: bool,
    jwks_refresh: std::time::Duration,
) -> (Router, OidcMock) {
    let mock = oidc::spawn();
    let app = super::TestApp::builder(db)
        .registration(registration)
        .google(&mock.url)
        .jwks_refresh(jwks_refresh)
        .build();
    (app, mock)
}

/// The `name` cookie a response set.
pub fn set_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| {
            v.strip_prefix(&prefix)
                .and_then(|rest| rest.split(';').next())
                .filter(|c| !c.is_empty())
                .map(str::to_string)
        })
}

pub fn location(headers: &HeaderMap) -> String {
    headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location")
        .to_string()
}

/// A query parameter of an absolute URL.
pub fn param(url: &str, name: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).expect("absolute URL");
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// A GET on `host` with optional cookies (`name=value` pairs).
pub async fn get(
    app: &Router,
    host: &str,
    uri: &str,
    cookies: &[String],
) -> (StatusCode, HeaderMap, Value) {
    let mut b = request(Method::GET, host, uri, super::auth::As::Nobody, None);
    if !cookies.is_empty() {
        b = b.header(header::COOKIE, cookies.join("; "));
    }
    exchange(app, b.body(Body::empty()).expect("request")).await
}

/// What `start` handed to "Google".
pub struct Started {
    pub host: String,
    pub state: String,
    pub nonce: String,
    pub challenge: String,
    /// `invoice_google_{12 hex}=…` (this flow's browser binding).
    pub browser: String,
}

/// `start` on `host` with `query`; must redirect to the mock's
/// authorization endpoint.
pub async fn start(
    app: &Router,
    mock: &OidcMock,
    host: &str,
    query: &str,
    cookies: &[String],
) -> Started {
    let (s, h, j) = get(
        app,
        host,
        &format!("/api/auth/google/start?{query}"),
        cookies,
    )
    .await;
    assert_eq!(s, StatusCode::FOUND, "{j}");
    let to = location(&h);
    assert!(to.starts_with(&format!("{}/auth?", mock.url)), "{to}");
    assert_eq!(param(&to, "client_id").as_deref(), Some(CLIENT_ID));
    assert_eq!(param(&to, "code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(param(&to, "prompt").as_deref(), Some("select_account"));
    assert_eq!(
        param(&to, "redirect_uri").as_deref(),
        Some("http://localhost:3000/api/auth/google/callback")
    );
    Started {
        host: host.to_string(),
        state: param(&to, "state").expect("state"),
        nonce: param(&to, "nonce").expect("nonce"),
        challenge: param(&to, "code_challenge").expect("challenge"),
        browser: binding_cookie(&h).expect("browser cookie"),
    }
}

/// The `invoice_google_…=value` pair a response set.
pub fn binding_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter(|v| v.starts_with("invoice_google_"))
        .find_map(|v| v.split(';').next().filter(|p| !p.ends_with('=')))
        .map(str::to_string)
}

/// `POST /api/account/google/link` as session `cookie` with the owner
/// password (and `code`); the status and body.
pub async fn link_request(
    app: &Router,
    host: &str,
    cookie: &str,
    code: Option<&str>,
) -> (StatusCode, Value) {
    let body = serde_json::json!({ "password": super::OWNER_PASSWORD, "code": code });
    super::auth::on(
        app,
        Method::POST,
        host,
        "/api/account/google/link",
        super::auth::As::Cookie(cookie),
        Some(body),
    )
    .await
}

/// The start query of a link (`purpose=link&grant=…`) for session `cookie`.
pub async fn link_query(app: &Router, host: &str, cookie: &str) -> String {
    let (s, j) = link_request(app, host, cookie, None).await;
    assert_eq!(s, StatusCode::OK, "{j}");
    let url = j["url"].as_str().expect("url");
    assert!(
        url.starts_with(&format!("http://{host}/api/auth/google/start?")),
        "{url}"
    );
    url.split_once('?')
        .map(|(_, q)| q.to_string())
        .expect("query")
}

/// Play Google: grant a code with `claims`, call the callback; returns the
/// redirect it answered.
pub async fn callback(app: &Router, mock: &OidcMock, st: &Started, claims: Value) -> String {
    let code = format!("code-{}", uuid::Uuid::new_v4());
    mock.grant(&code, &st.challenge, claims);
    callback_raw(app, &format!("state={}&code={code}", st.state)).await
}

pub async fn callback_raw(app: &Router, query: &str) -> String {
    let (s, h, j) = get(
        app,
        BASE_HOST,
        &format!("/api/auth/google/callback?{query}"),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::FOUND, "{j}");
    location(&h)
}

/// The result of following a handoff.
pub struct Finished {
    pub location: String,
    pub session: Option<String>,
    pub pending: Option<String>,
}

/// Follow `to` (the callback's redirect to `…/finish?token=…`) on `host`
/// with `cookies`.
pub async fn finish(app: &Router, to: &str, host: &str, cookies: &[String]) -> Finished {
    let path = to
        .split_once("/api/auth/google/finish")
        .map(|(_, rest)| format!("/api/auth/google/finish{rest}"))
        .unwrap_or_else(|| panic!("not a handoff: {to}"));
    let (s, h, j) = get(app, host, &path, cookies).await;
    assert_eq!(s, StatusCode::FOUND, "{j}");
    Finished {
        location: location(&h),
        session: session_cookie(&h),
        pending: pending_cookie(&h),
    }
}

/// start → callback (with `sub` / `email`) → finish on the same host.
pub async fn sign_in(
    app: &Router,
    mock: &OidcMock,
    host: &str,
    query: &str,
    session: Option<&str>,
    sub: &str,
    email: &str,
) -> Finished {
    let cookies: Vec<String> = session
        .map(|c| format!("invoice_session={c}"))
        .into_iter()
        .collect();
    let st = start(app, mock, host, query, &cookies).await;
    let to = callback(app, mock, &st, oidc::claims(mock, sub, email, &st.nonce)).await;
    if !to.contains("/api/auth/google/finish?token=") {
        return Finished {
            location: to,
            session: None,
            pending: None,
        };
    }
    let mut cookies = cookies;
    cookies.push(st.browser.clone());
    finish(app, &to, host, &cookies).await
}

/// `http://{host}{path}`.
pub fn at(host: &str, path: &str) -> String {
    format!("http://{host}{path}")
}
