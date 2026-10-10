//! Spaces, users, tokens and sessions for tests: seeding straight through
//! the library (no HTTP), and requests with an explicit host, token, cookie
//! and `Origin`.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use invoice::auth::entity::api_token;
use invoice::auth::{crypto, users};
use invoice::space::{Role, SpaceId, repo as space_repo};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use serde_json::Value;
use uuid::Uuid;

use super::{OWNER_PASSWORD, send};

/// A verified user with [`OWNER_PASSWORD`].
pub async fn user(conn: &DatabaseConnection, email: &str) -> Uuid {
    let hash = crypto::hash_password(OWNER_PASSWORD.into())
        .await
        .expect("hash password");
    let u = users::create(conn, email, "Test User", Some(hash))
        .await
        .expect("create user")
        .expect("a new e-mail");
    users::mark_verified(conn, u.id).await.expect("verify");
    u.id
}

/// A space `slug` owned by a new verified user `email`, seeded like a
/// space created through the API; `token` (API token format) is stored as
/// that owner's token with role owner.
pub async fn seed_space(
    conn: &DatabaseConnection,
    slug: &str,
    email: &str,
    token: Option<&str>,
) -> (SpaceId, Uuid) {
    let owner = user(conn, email).await;
    let space = space_repo::create(conn, slug.into(), format!("Space {slug}"), owner)
        .await
        .expect("create space");
    let space = SpaceId(space.id);
    if let Some(t) = token {
        store_token(conn, space, owner, Role::Owner, t).await;
    }
    (space, owner)
}

/// Store `token` (shape `inv_{8 hex}_{secret}`) for `user` in `space`.
pub async fn store_token(
    conn: &DatabaseConnection,
    space: SpaceId,
    user: Uuid,
    role: Role,
    token: &str,
) {
    api_token::ActiveModel {
        id: Set(Uuid::new_v4()),
        space_id: Set(space.uuid()),
        user_id: Set(user),
        name: Set("test".into()),
        prefix: Set(token[4..12].to_string()),
        token_hash: Set(crypto::digest(token)),
        role: Set(role.as_str().into()),
        created_at: Set(chrono::Utc::now().into()),
        expires_at: Set(None),
        last_used_at: Set(None),
    }
    .insert(conn)
    .await
    .expect("store token");
}

/// A member `email` of `space` with `role` (the row is written directly —
/// simpler than the invitation flow, which `tests/invites*.rs` cover) and a
/// token of that role; returns the token.
pub async fn member(conn: &DatabaseConnection, space: SpaceId, email: &str, role: Role) -> String {
    let id = user(conn, email).await;
    invoice::space::entity::member::ActiveModel {
        space_id: Set(space.uuid()),
        user_id: Set(id),
        role: Set(role.as_str().into()),
        created_at: Set(chrono::Utc::now().into()),
    }
    .insert(conn)
    .await
    .expect("add member");
    let (token, _) = crypto::new_api_token().expect("token");
    store_token(conn, space, id, role, &token).await;
    token
}

/// How a request authenticates.
#[derive(Clone, Copy)]
pub enum As<'a> {
    Nobody,
    Bearer(&'a str),
    /// The `invoice_session` cookie value; mutations get the host's own `Origin`.
    Cookie(&'a str),
}

/// `http://{host}` — the origin a browser on that host sends.
pub fn origin(host: &str) -> String {
    format!("http://{host}")
}

pub fn request(
    method: Method,
    host: &str,
    uri: &str,
    who: As<'_>,
    body: Option<Value>,
) -> axum::http::request::Builder {
    let mut b = Request::builder()
        .method(method.clone())
        .uri(uri)
        .header(header::HOST, host);
    match who {
        As::Nobody => {}
        As::Bearer(t) => b = b.header(header::AUTHORIZATION, format!("Bearer {t}")),
        As::Cookie(c) => {
            b = b.header(header::COOKIE, format!("invoice_session={c}"));
            if method != Method::GET {
                b = b.header(header::ORIGIN, origin(host));
            }
        }
    }
    if body.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/json");
    }
    b
}

/// Send and read the status, headers and JSON body (`Null` when empty).
pub async fn exchange(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let resp = send(app.clone(), req).await;
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, headers, json)
}

/// One call on `host` as `who`.
pub async fn on(
    app: &Router,
    method: Method,
    host: &str,
    uri: &str,
    who: As<'_>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let b = request(method, host, uri, who, body.clone());
    let req = match body {
        Some(v) => b.body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .expect("build request");
    let (status, _, json) = exchange(app, req).await;
    (status, json)
}

/// The `invoice_session` value a response set.
pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| {
            v.strip_prefix("invoice_session=")
                .and_then(|rest| rest.split(';').next())
                .filter(|c| !c.is_empty())
                .map(str::to_string)
        })
}

/// Log in on `host`; the session cookie, or the failing status.
pub async fn login(
    app: &Router,
    host: &str,
    email: &str,
    password: &str,
) -> Result<String, (StatusCode, Value)> {
    let body = serde_json::json!({ "email": email, "password": password });
    let req = request(
        Method::POST,
        host,
        "/api/auth/login",
        As::Nobody,
        Some(body.clone()),
    )
    .body(Body::from(body.to_string()))
    .expect("build request");
    let (status, headers, json) = exchange(app, req).await;
    match (status, session_cookie(&headers)) {
        (StatusCode::NO_CONTENT, Some(c)) => Ok(c),
        _ => Err((status, json)),
    }
}

/// The app with the mock SMTP configured and registration `on`.
pub fn mail_app(
    db: &DatabaseConnection,
    env: &super::smtp::EmailEnv,
    registration: bool,
) -> Router {
    let pdf = super::pdf_service(&env.pdf.url, env.pdf.storage.storage.clone());
    let mut state = super::state(
        db.clone(),
        invoice::ares::DEFAULT_ARES_URL,
        invoice::cnb::DEFAULT_CNB_URL,
        pdf,
    );
    state.email = Some(invoice::email::Mailer::new(&env.config).expect("mailer"));
    state.registration = registration;
    super::app(state)
}

/// The text body of a single-part mail (7bit / 8bit, quoted-printable or
/// base64 transfer encoding).
pub fn mail_text(mail: &super::smtp::Mail) -> String {
    use base64::Engine as _;
    let body = mail.data.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    let cte = mail
        .header("content-transfer-encoding")
        .unwrap_or_default()
        .to_ascii_lowercase();
    match cte.as_str() {
        "base64" => {
            let b64: String = body.split_whitespace().collect();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .expect("base64 body");
            String::from_utf8(bytes).expect("utf-8 body")
        }
        "quoted-printable" => {
            let joined = body.replace("=\r\n", "");
            let mut out = Vec::new();
            let b = joined.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'=' && i + 2 < b.len() {
                    let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                    if let Ok(v) = u8::from_str_radix(hex, 16) {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                }
                out.push(b[i]);
                i += 1;
            }
            String::from_utf8(out).expect("utf-8 body")
        }
        _ => body.to_string(),
    }
}

/// The token of the `{page}?token=…` link in `mail`.
pub fn mail_token(mail: &super::smtp::Mail, page: &str) -> String {
    let text = mail_text(mail);
    let marker = format!("/{page}?token=");
    let start = text
        .find(&marker)
        .unwrap_or_else(|| panic!("no {marker} link in: {text}"))
        + marker.len();
    text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// The status of a call on the default space host with `token` (a 403 must
/// carry `{"code":"forbidden"}`).
pub async fn status(
    app: &Router,
    m: Method,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> StatusCode {
    let (s, j) = on(app, m, super::TEST_HOST, uri, As::Bearer(token), body).await;
    if s == StatusCode::FORBIDDEN {
        assert_eq!(j, serde_json::json!({ "code": "forbidden" }), "{uri}");
    }
    s
}

/// The password of users registered through the API in tests.
pub const PASSWORD: &str = "a long enough password";

/// A valid registration body.
pub fn register_body(email: &str) -> Value {
    serde_json::json!({ "email": email, "password": PASSWORD, "displayName": "Jana Nováková" })
}

/// The `n`-th (1-based) captured mail, waiting for the background send.
pub async fn mail(smtp: &super::smtp::SmtpMock, n: usize) -> super::smtp::Mail {
    for _ in 0..500 {
        if let Some(m) = smtp.mails().get(n - 1) {
            return m.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("mail #{n} never arrived ({} captured)", smtp.mails().len());
}

/// Give background sends time to happen before asserting that none did.
pub async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}
