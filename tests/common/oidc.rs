//! A local mock OpenID provider (google.md, 4d) — never the real Google:
//! discovery, a JWKS with the fictitious test-only RSA keys in
//! `tests/fixtures/oidc/` (generated for these tests, used nowhere else),
//! and a token endpoint that checks the client secret and PKCE and answers
//! an ID token signed with the claims a test registered for its code.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const CLIENT_ID: &str = "test-client.apps.example";
pub const CLIENT_SECRET: &str = "test-client-secret";
pub const KEY_A: &str = include_str!("../fixtures/oidc/key_a.pem");
pub const KEY_B: &str = include_str!("../fixtures/oidc/key_b.pem");
const JWK_A: &str = include_str!("../fixtures/oidc/jwk_a.json");
const JWK_B: &str = include_str!("../fixtures/oidc/jwk_b.json");

struct Grant {
    claims: Value,
    challenge: String,
}

struct Inner {
    issuer: String,
    grants: HashMap<String, Grant>,
    /// `(pem, kid)` the ID tokens are signed with.
    signer: (&'static str, &'static str),
    /// The published JWKS keys.
    published: Vec<&'static str>,
    /// `Some(status)`: the token endpoint fails with it.
    token_status: Option<u16>,
    jwks_fetches: usize,
    token_requests: usize,
    /// `Cache-Control` of the JWKS response.
    jwks_cache_control: Option<&'static str>,
}

#[derive(Clone)]
pub struct OidcMock {
    pub url: String,
    inner: Arc<Mutex<Inner>>,
}

pub fn jwk_a() -> Value {
    serde_json::from_str(JWK_A).expect("jwk a")
}

impl OidcMock {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("mock OIDC state")
    }

    /// Answer `code` with an ID token of `claims` (when the verifier
    /// matches `challenge`).
    pub fn grant(&self, code: &str, challenge: &str, claims: Value) {
        self.lock().grants.insert(
            code.into(),
            Grant {
                claims,
                challenge: challenge.into(),
            },
        );
    }

    /// Sign with `pem` under `kid` from now on.
    pub fn sign_with(&self, pem: &'static str, kid: &'static str) {
        self.lock().signer = (pem, kid);
    }

    /// Publish key A only (`false`) or keys A and B (`true`).
    pub fn publish_b(&self, on: bool) {
        self.lock().published = if on { vec![JWK_A, JWK_B] } else { vec![JWK_A] };
    }

    pub fn fail_token(&self, status: Option<u16>) {
        self.lock().token_status = status;
    }

    pub fn jwks_cache_control(&self, value: Option<&'static str>) {
        self.lock().jwks_cache_control = value;
    }

    pub fn jwks_fetches(&self) -> usize {
        self.lock().jwks_fetches
    }

    pub fn token_requests(&self) -> usize {
        self.lock().token_requests
    }

    pub fn issuer(&self) -> String {
        self.lock().issuer.clone()
    }
}

async fn discovery(State(m): State<OidcMock>) -> Json<Value> {
    let iss = m.issuer();
    Json(json!({
        "issuer": iss,
        "authorization_endpoint": format!("{iss}/auth"),
        "token_endpoint": format!("{iss}/token"),
        "jwks_uri": format!("{iss}/jwks"),
    }))
}

async fn jwks(State(m): State<OidcMock>) -> Response {
    let mut inner = m.lock();
    inner.jwks_fetches += 1;
    let keys: Vec<Value> = inner
        .published
        .iter()
        .map(|k| serde_json::from_str(k).expect("jwk"))
        .collect();
    let mut resp = Json(json!({ "keys": keys })).into_response();
    if let Some(cc) = inner.jwks_cache_control {
        resp.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static(cc),
        );
    }
    resp
}

fn oauth_error(error: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
}

async fn token(State(m): State<OidcMock>, Form(f): Form<HashMap<String, String>>) -> Response {
    let mut inner = m.lock();
    inner.token_requests += 1;
    if let Some(status) = inner.token_status {
        let status = StatusCode::from_u16(status).expect("status");
        return (status, Json(json!({ "error": "server_error" }))).into_response();
    }
    let field = |k: &str| f.get(k).map(String::as_str).unwrap_or_default();
    if field("client_id") != CLIENT_ID || field("client_secret") != CLIENT_SECRET {
        return oauth_error("invalid_client");
    }
    if field("grant_type") != "authorization_code" {
        return oauth_error("unsupported_grant_type");
    }
    let Some(grant) = inner.grants.remove(field("code")) else {
        return oauth_error("invalid_grant");
    };
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(field("code_verifier").as_bytes()));
    if challenge != grant.challenge {
        return oauth_error("invalid_grant");
    }
    let (pem, kid) = inner.signer;
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.into());
    let key = EncodingKey::from_rsa_pem(pem.as_bytes()).expect("test key");
    let id_token = encode(&header, &grant.claims, &key).expect("sign");
    Json(json!({ "access_token": "mock-access", "token_type": "Bearer", "id_token": id_token }))
        .into_response()
}

/// Start the mock on a free port (inside the current Tokio runtime).
pub fn spawn() -> OidcMock {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock OIDC");
    listener
        .set_nonblocking(true)
        .expect("non-blocking listener");
    let url = format!("http://{}", listener.local_addr().expect("mock address"));
    let mock = OidcMock {
        url: url.clone(),
        inner: Arc::new(Mutex::new(Inner {
            issuer: url,
            grants: HashMap::new(),
            signer: (KEY_A, "test-a"),
            published: vec![JWK_A],
            token_status: None,
            jwks_fetches: 0,
            token_requests: 0,
            jwks_cache_control: None,
        })),
    };
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .with_state(mock.clone());
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
        axum::serve(listener, app).await.expect("mock OIDC server");
    });
    mock
}

/// ID token claims for `email` / `sub`, verified, issued now.
pub fn claims(mock: &OidcMock, sub: &str, email: &str, nonce: &str) -> Value {
    let now = chrono::Utc::now().timestamp();
    json!({
        "iss": mock.issuer(),
        "aud": CLIENT_ID,
        "sub": sub,
        "iat": now,
        "exp": now + 3600,
        "nonce": nonce,
        "email": email,
        "email_verified": true,
        "name": "Jana Googlová",
    })
}
