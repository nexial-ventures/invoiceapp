//! Shared integration-test harness.
//!
//! Each [`TestDb`] owns a unique throwaway Postgres schema (`test_<n>`) on the
//! server named by `TEST_DATABASE_URL`, runs every migration into it, and
//! drops it on teardown — tests are parallel-safe with no shared state.
//!
//! Every `TestDb` starts with one space (`acme`, [`TEST_HOST`]) owned by a
//! verified user holding the owner API token [`TEST_TOKEN`]; the routers
//! send requests without a `Host` header to that space's host, so a test
//! written before spaces runs inside it unchanged.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, Response, StatusCode, header};
use invoice::app::{self, AppState};
use invoice::ares::{AresClient, DEFAULT_ARES_URL};
use invoice::auth::host::PublicUrl;
use invoice::auth::ratelimit::RateLimiter;
use invoice::cnb::{CnbClient, DEFAULT_CNB_URL};
use invoice::migration::{Migrator, MigratorTrait};
use invoice::pdf::PdfRoot;
use invoice::space::SpaceId;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use tower::ServiceExt;

pub mod auth;
pub mod csv_export;
pub mod csvio;
pub mod design;
pub mod documents;
pub mod email_limits;
pub mod google;
pub mod isdoc;
pub mod limits;
pub mod mcp;
pub mod mdcast;
pub mod members;
pub mod mfa;
pub mod money;
pub mod oidc;
pub mod pohoda;
pub mod received;
pub mod smtp;
pub mod storage;

/// Owner token of the default space (API token format `inv_{prefix}_{secret}`).
pub const TEST_TOKEN: &str = "inv_7e57c0de_dGVzdC10b2tlbi1zZWNyZXQtMDEyMzQ1Njc4OWFiY2RlZg";
/// `INVOICE__PUBLIC_URL` of every test app.
pub const PUBLIC_URL: &str = "http://localhost:3000";
pub const BASE_HOST: &str = "localhost:3000";
pub const TEST_SLUG: &str = "acme";
/// The default space's host (what a request without `Host` goes to).
pub const TEST_HOST: &str = "acme.localhost:3000";
pub const OWNER_EMAIL: &str = "owner@example.com";
pub const OWNER_PASSWORD: &str = "correct horse battery staple";

pub fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL").expect(
        "TEST_DATABASE_URL must be set for integration tests \
         (local Postgres, see .env.example)",
    )
}

pub fn unique_schema() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        "test_{}_{}_{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

pub struct TestDb {
    pub conn: DatabaseConnection,
    pub schema: String,
    base_url: String,
    /// The default space (`acme`).
    pub space: SpaceId,
    /// Its owner (holds [`TEST_TOKEN`]).
    pub user_id: uuid::Uuid,
}

impl TestDb {
    pub async fn new() -> Self {
        let base_url = test_database_url();
        let schema = unique_schema();

        let admin = Database::connect(&base_url).await.expect(
            "connect to TEST_DATABASE_URL (is the local Postgres running? see .env.example)",
        );
        admin
            .execute_unprepared(&format!("CREATE SCHEMA \"{schema}\""))
            .await
            .expect("create test schema");
        admin.close().await.expect("close admin connection");

        let mut opts = ConnectOptions::new(base_url.clone());
        opts.set_schema_search_path(schema.clone())
            .max_connections(5)
            .sqlx_logging(false);
        let conn = Database::connect(opts)
            .await
            .expect("connect to test schema");
        Migrator::up(&conn, None).await.expect("run migrations");
        let (space, user_id) =
            auth::seed_space(&conn, TEST_SLUG, OWNER_EMAIL, Some(TEST_TOKEN)).await;

        Self {
            conn,
            schema,
            base_url,
            space,
            user_id,
        }
    }

    /// The storage key of `rel` in the default space.
    pub fn key(&self, rel: &str) -> String {
        format!("{}/{rel}", self.space.storage_prefix())
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // Dedicated thread + runtime: works under any test runtime flavor.
        let url = self.base_url.clone();
        let schema = self.schema.clone();
        let _ = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                if let Ok(conn) = Database::connect(&url).await {
                    // This blocks the test's runtime, so a lock held by one of
                    // the test's own connections could never be released:
                    // give up (leaking the schema) instead of hanging forever.
                    let _ = conn
                        .execute_unprepared(&format!(
                            "SET lock_timeout = '10s'; DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"
                        ))
                        .await;
                }
            });
        })
        .join();
    }
}

/// App router; ARES and ČNB point at the real default URLs, which tests never call.
pub fn router(db: DatabaseConnection) -> Router {
    router_with_ares(db, DEFAULT_ARES_URL)
}

/// [`router`] with `INVOICE__REGISTRATION` on, so `POST /api/spaces` is allowed.
pub fn router_open(db: DatabaseConnection) -> Router {
    let (mdcast_url, _) = mdcast::spawn();
    let mut state = state(
        db,
        DEFAULT_ARES_URL,
        DEFAULT_CNB_URL,
        pdf_service(&mdcast_url, shared_storage()),
    );
    state.registration = true;
    app(state)
}

/// App router with ARES at `ares_url` (a local mock server in tests).
pub fn router_with_ares(db: DatabaseConnection, ares_url: &str) -> Router {
    router_with(db, ares_url, DEFAULT_CNB_URL)
}

/// App router with ČNB at `cnb_url` (a local mock server in tests).
pub fn router_with_cnb(db: DatabaseConnection, cnb_url: &str) -> Router {
    router_with(db, DEFAULT_ARES_URL, cnb_url)
}

/// Every router renders PDFs against its own mock mdcast (never the real
/// service) into a storage dir shared by the test binary — archive paths
/// contain the document id, so tests cannot collide.
pub fn router_with(db: DatabaseConnection, ares_url: &str, cnb_url: &str) -> Router {
    let (mdcast_url, _) = mdcast::spawn();
    app(state(
        db,
        ares_url,
        cnb_url,
        pdf_service(&mdcast_url, shared_storage()),
    ))
}

/// The app of `state`; a request without `Host` goes to [`TEST_HOST`].
pub fn app(state: AppState) -> Router {
    app::router(state).layer(axum::middleware::map_request(default_host))
}

async fn default_host(mut req: Request) -> Request {
    if !req.headers().contains_key(header::HOST) {
        req.headers_mut()
            .insert(header::HOST, HeaderValue::from_static(TEST_HOST));
    }
    req
}

/// One fs storage per test binary; tests never write `design/` into it.
pub fn shared_storage() -> invoice::storage::Storage {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("storage");
    invoice::storage::Storage::local(&dir).expect("shared fs storage")
}

pub fn pdf_service(mdcast_url: &str, storage: invoice::storage::Storage) -> PdfRoot {
    PdfRoot::new(mdcast_url, None, storage).expect("build PDF service")
}

/// The PDF service of `space` (tests calling the archive / import code directly).
pub fn space_pdf(
    mdcast_url: &str,
    storage: invoice::storage::Storage,
    space: SpaceId,
) -> invoice::pdf::PdfService {
    pdf_service(mdcast_url, storage)
        .space(space)
        .expect("space PDF service")
}

pub fn state(db: DatabaseConnection, ares_url: &str, cnb_url: &str, pdf: PdfRoot) -> AppState {
    AppState {
        db,
        ares: AresClient::new(ares_url).expect("build ARES client"),
        cnb: CnbClient::new(cnb_url).expect("build ČNB client"),
        pdf,
        email: None,
        public: PublicUrl::parse(PUBLIC_URL).expect("public URL"),
        email_disabled: false,
        registration: false,
        max_spaces_per_user: invoice::config::DEFAULT_MAX_SPACES_PER_USER,
        trust_forwarded: false,
        secret_key: test_secret_key(),
        limiter: RateLimiter::default(),
        google: None,
    }
}

/// `TEST_SECRET_KEY` (base64, 32 bytes) or a fixed, fictitious test key.
pub fn test_secret_key() -> invoice::secret::SecretKey {
    match std::env::var("TEST_SECRET_KEY") {
        Ok(raw) if !raw.trim().is_empty() => {
            invoice::secret::SecretKey::parse(&raw).expect("TEST_SECRET_KEY: 32 bytes, base64")
        }
        _ => invoice::secret::SecretKey::from_bytes(*b"invoice-test-secret-key-32-bytes"),
    }
}

pub async fn send(app: Router, req: Request<Body>) -> Response<Body> {
    app.oneshot(req).await.expect("infallible router")
}

pub fn get(uri: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri);
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::empty()).expect("build request")
}

pub async fn json(resp: Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

/// Authenticated request with an optional JSON body.
pub fn authed(method: Method, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let b = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {TEST_TOKEN}"));
    match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .expect("build request")
}

/// Send an authenticated request; returns the status and the JSON body
/// (`Null` for an empty body).
pub async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let resp = send(app.clone(), authed(method, uri, body)).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON body")
    };
    (status, json)
}

/// `xmllint --noout --schema tests/fixtures/{xsd}` over `xml` (the vendored
/// ISDOC / Pohoda / Money S3 schemas); a failure shows the errors and
/// `shown` (the document as text).
pub fn assert_xsd(xml: &[u8], xsd: &str, shown: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("doc.xml");
    std::fs::write(&file, xml).expect("write xml");
    let xsd = format!("{}/tests/fixtures/{xsd}", env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new("xmllint")
        .args(["--noout", "--schema", &xsd])
        .arg(&file)
        .output()
        .expect("run xmllint (libxml2-utils) — needed for the XML schema checks");
    assert!(
        out.status.success(),
        "{}\n{shown}",
        String::from_utf8_lossy(&out.stderr)
    );
}
