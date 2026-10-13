//! TOTP (mfa.md, 4c): enrolment through the API, valid codes computed from
//! the secret the setup returned, the login's code step.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode, header};
use invoiceapp::auth::mfa::totp;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use super::auth::{As, exchange, login, on, request, session_cookie};
use super::{BASE_HOST, OWNER_PASSWORD};

/// RFC 4648 base32 (no padding) → bytes.
pub fn base32_decode(s: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut buf, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        let v = ALPHABET.iter().position(|a| *a == c).expect("base32 char") as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

pub fn now_step() -> i64 {
    totp::step_at(chrono::Utc::now().timestamp())
}

/// The current code of `secret`.
pub fn code_now(secret: &[u8]) -> String {
    totp::code(secret, now_step())
}

/// Forget the user's last accepted step (the replay guard), then the
/// current code: a code that will be accepted again.
pub async fn fresh(conn: &DatabaseConnection, user: Uuid, secret: &[u8]) -> String {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "UPDATE users SET totp_last_step = NULL WHERE id = $1",
        vec![user.into()],
    ))
    .await
    .expect("reset last step");
    code_now(secret)
}

pub struct Enrolled {
    pub secret: Vec<u8>,
    pub codes: Vec<String>,
    /// The base-host session used for the enrolment.
    pub cookie: String,
}

/// Log in on the base host (no TOTP yet) and enable TOTP.
pub async fn enrol(app: &Router, email: &str) -> Enrolled {
    let cookie = login(app, BASE_HOST, email, OWNER_PASSWORD)
        .await
        .expect("base-host login");
    let (s, j) = on(
        app,
        Method::POST,
        BASE_HOST,
        "/api/account/mfa/setup",
        As::Cookie(&cookie),
        Some(json!({ "password": OWNER_PASSWORD })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{j}");
    let secret = base32_decode(j["secret"].as_str().expect("secret"));
    let (s, j) = on(
        app,
        Method::POST,
        BASE_HOST,
        "/api/account/mfa/enable",
        As::Cookie(&cookie),
        Some(json!({ "code": code_now(&secret) })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{j}");
    let codes = j["recoveryCodes"]
        .as_array()
        .expect("codes")
        .iter()
        .map(|c| c.as_str().expect("code").to_string())
        .collect();
    Enrolled {
        secret,
        codes,
        cookie,
    }
}

/// The `invoice_mfa` value a response set.
pub fn pending_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| {
            v.strip_prefix("invoice_mfa=")
                .and_then(|rest| rest.split(';').next())
                .filter(|c| !c.is_empty())
                .map(str::to_string)
        })
}

/// `POST /api/auth/login` of a user with TOTP: the pending cookie.
pub async fn start(app: &Router, host: &str, email: &str) -> String {
    let body = json!({ "email": email, "password": OWNER_PASSWORD });
    let req = request(
        Method::POST,
        host,
        "/api/auth/login",
        As::Nobody,
        Some(body.clone()),
    )
    .body(Body::from(body.to_string()))
    .expect("build request");
    let (s, headers, j) = exchange(app, req).await;
    assert_eq!((s, j), (StatusCode::OK, json!({ "mfa": "required" })));
    assert!(session_cookie(&headers).is_none());
    pending_cookie(&headers).expect("invoice_mfa cookie")
}

/// `POST /api/auth/login/mfa` with `pending`: status, session cookie, body,
/// headers.
pub async fn finish(
    app: &Router,
    host: &str,
    pending: Option<&str>,
    code: &str,
) -> (StatusCode, Option<String>, Value, HeaderMap) {
    let body = json!({ "code": code });
    let mut b = request(
        Method::POST,
        host,
        "/api/auth/login/mfa",
        As::Nobody,
        Some(body.clone()),
    );
    if let Some(p) = pending {
        b = b.header(header::COOKIE, format!("invoice_mfa={p}"));
    }
    let req = b.body(Body::from(body.to_string())).expect("build request");
    let (s, headers, j) = exchange(app, req).await;
    (s, session_cookie(&headers), j, headers)
}

/// Turn the TOTP policy of the default space on directly (no API).
pub async fn require_mfa(conn: &DatabaseConnection, slug: &str) {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "UPDATE spaces SET require_mfa = true WHERE slug = $1",
        vec![slug.into()],
    ))
    .await
    .expect("set policy");
}
