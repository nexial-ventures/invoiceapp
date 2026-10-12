//! A local mock of the mdcast endpoints `render_template` uses: the render
//! (with the `409` → `POST /v1/blobs` → retry negotiation) — never the real
//! service. Records what each render received.
//!
//! A template containing `// mock-fail` fails like a template that does not
//! compile; `// mock-fail-en` only for the `en` sample (design checks).
//! [`MdcastMock::delay`] delays every render; [`MdcastMock::hold`] holds
//! them until [`MdcastMock::release`] (deterministic "during a render").

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use invoice::pdf::PdfRoot;
use serde_json::{Value, json};

/// One successful render as the mock saw it.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// The parsed `/data.json`.
    pub data: Value,
    pub template: String,
    /// Asset manifest keys, sorted.
    pub assets: Vec<String>,
    /// Asset manifest key → content digest (tells same-named files apart).
    pub digests: std::collections::BTreeMap<String, String>,
    pub fonts: Vec<String>,
    /// The bytes answered.
    pub pdf: Vec<u8>,
}

#[derive(Default)]
struct Inner {
    known: HashSet<String>,
    renders: Vec<Rendered>,
    uploads: usize,
    /// `Some((status, code, message))` → every render fails with that error.
    fail: Option<(u16, String, String)>,
    /// Render requests received (before any delay or failure).
    started: usize,
    delay: Option<std::time::Duration>,
    /// Set by `hold`: renders wait for a permit (`release` adds plenty).
    gate: Option<Arc<tokio::sync::Semaphore>>,
}

#[derive(Clone, Default)]
pub struct MdcastMock(Arc<Mutex<Inner>>);

impl MdcastMock {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().expect("mock mdcast state")
    }

    pub fn renders(&self) -> Vec<Rendered> {
        self.lock().renders.clone()
    }

    pub fn last(&self) -> Rendered {
        self.renders().pop().expect("a render reached the mock")
    }

    pub fn count(&self) -> usize {
        self.lock().renders.len()
    }

    pub fn uploads(&self) -> usize {
        self.lock().uploads
    }

    /// Renders fail like a template that does not compile.
    pub fn fail_with(&self, message: &str) {
        self.fail_as(422, "render_failed", message);
    }

    pub fn fail_as(&self, status: u16, code: &str, message: &str) {
        self.lock().fail = Some((status, code.to_string(), message.to_string()));
    }

    /// Every render answers only after `d`.
    pub fn delay(&self, d: std::time::Duration) {
        self.lock().delay = Some(d);
    }

    /// Every render (after counting as started) waits until [`Self::release`].
    pub fn hold(&self) {
        self.lock().gate = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    }

    /// Let held and later renders through.
    pub fn release(&self) {
        if let Some(gate) = &self.lock().gate {
            gate.add_permits(tokio::sync::Semaphore::MAX_PERMITS / 2);
        }
    }

    /// Render requests received so far (also failed or still running ones).
    pub fn started(&self) -> usize {
        self.lock().started
    }

    /// Wait until `n` render requests have arrived (5 s at most).
    pub async fn wait_started(&self, n: usize) {
        for _ in 0..500 {
            if self.started() >= n {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("only {} of {n} renders started", self.started());
    }
}

async fn render(State(m): State<MdcastMock>, body: Bytes) -> Response {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let body = json!({ "code": "bad_request", "message": e.to_string() });
            return (StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
        }
    };
    // The blob negotiation's first request (409) does not count as a start.
    let first_try = !req["assets"].as_object().is_some_and(|a| {
        let inner = m.lock();
        a.values()
            .all(|d| d.as_str().is_some_and(|d| inner.known.contains(d)))
    });
    let (delay, gate) = {
        let mut inner = m.lock();
        if !first_try {
            inner.started += 1;
        }
        (inner.delay, inner.gate.clone())
    };
    if let Some(d) = delay.filter(|_| !first_try) {
        tokio::time::sleep(d).await;
    }
    if let Some(gate) = gate.filter(|_| !first_try) {
        let _permit = gate.acquire().await.expect("mock gate open");
    }
    let mut inner = m.lock();
    if let Some((status, code, message)) = inner.fail.clone() {
        let body = json!({ "code": code, "message": message });
        let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return (status, axum::Json(body)).into_response();
    }
    let template = req["template_source"].as_str().unwrap_or_default();
    let en = req["data"]
        .as_str()
        .is_some_and(|d| d.contains("\"locale\":\"en\""));
    if template.contains("// mock-fail-en") && en
        || template.contains("// mock-fail") && !template.contains("// mock-fail-en")
    {
        let body = json!({ "code": "render_failed", "message": "error: mock template failure" });
        return (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(body)).into_response();
    }
    let assets = req["assets"].as_object().cloned().unwrap_or_default();
    let missing: Vec<Value> = assets
        .iter()
        .filter_map(|(key, digest)| {
            let d = digest.as_str()?;
            (!inner.known.contains(d)).then(|| json!({ "key": key, "digest": d }))
        })
        .collect();
    if !missing.is_empty() {
        let body = json!({ "message": "blobs missing", "missing": missing });
        return (StatusCode::CONFLICT, axum::Json(body)).into_response();
    }
    let pdf = format!("%PDF-1.7 mock render {}\n", inner.renders.len() + 1).into_bytes();
    let mut keys: Vec<String> = assets.keys().cloned().collect();
    keys.sort();
    inner.renders.push(Rendered {
        data: serde_json::from_str(req["data"].as_str().unwrap_or("null")).unwrap_or(Value::Null),
        template: req["template_source"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        assets: keys,
        digests: assets
            .iter()
            .filter_map(|(k, d)| Some((k.clone(), d.as_str()?.to_string())))
            .collect(),
        fonts: req["fonts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        pdf: pdf.clone(),
    });
    (
        [
            (header::CONTENT_TYPE, "application/pdf"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"output.pdf\"",
            ),
        ],
        pdf,
    )
        .into_response()
}

/// Multipart upload: each part's name is the digest it carries.
async fn blobs(State(m): State<MdcastMock>, body: Bytes) -> StatusCode {
    let text = String::from_utf8_lossy(&body);
    let mut inner = m.lock();
    inner.uploads += 1;
    for part in text.split("name=\"").skip(1) {
        if let Some((digest, _)) = part.split_once('"') {
            inner.known.insert(digest.to_string());
        }
    }
    StatusCode::NO_CONTENT
}

/// Start the mock on a free port (inside the current Tokio runtime); returns
/// its base URL and the recorder.
pub fn spawn() -> (String, MdcastMock) {
    let mock = MdcastMock::default();
    let app = Router::new()
        .route("/v1/render/template", post(render))
        .route("/v1/blobs", post(blobs))
        .with_state(mock.clone());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock mdcast");
    listener
        .set_nonblocking(true)
        .expect("non-blocking listener");
    let addr = listener.local_addr().expect("mock address");
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
        axum::serve(listener, app)
            .await
            .expect("mock mdcast server");
    });
    (format!("http://{addr}"), mock)
}

/// A mock mdcast plus a private storage, for tests that inspect both.
pub struct PdfEnv {
    pub url: String,
    pub mock: MdcastMock,
    pub storage: super::storage::TestStorage,
}

impl PdfEnv {
    /// fs storage in a throwaway directory.
    pub fn new() -> Self {
        Self::with_storage(super::storage::TestStorage::fs())
    }

    pub fn with_storage(storage: super::storage::TestStorage) -> Self {
        let (url, mock) = spawn();
        Self { url, mock, storage }
    }

    /// The app rendering against this mock.
    pub fn router(&self, db: sea_orm::DatabaseConnection) -> Router {
        self.router_at(db, &self.url)
    }

    /// Same storage, but mdcast at `url` (e.g. a dead one).
    pub fn router_at(&self, db: sea_orm::DatabaseConnection, url: &str) -> Router {
        self.router_with(db, url, self.storage.storage.clone())
    }

    /// Same mock, any storage (e.g. an unreachable one).
    pub fn router_with(
        &self,
        db: sea_orm::DatabaseConnection,
        url: &str,
        storage: invoice::storage::Storage,
    ) -> Router {
        let pdf = super::pdf_service(url, storage);
        Self::app(db, pdf)
    }

    /// The app rendering against this mock under `limits` (short waits).
    pub fn router_limited(
        &self,
        db: sea_orm::DatabaseConnection,
        limits: invoice::pdf::limits::RenderLimits,
    ) -> Router {
        let pdf = super::pdf_service(&self.url, self.storage.storage.clone());
        Self::app(db, pdf.with_render_limits(limits))
    }

    /// The app rendering against this mock, plus its PDF service (its
    /// shared design cache is inspectable).
    pub fn router_and_pdf(&self, db: sea_orm::DatabaseConnection) -> (Router, PdfRoot) {
        let pdf = super::pdf_service(&self.url, self.storage.storage.clone());
        (Self::app(db, pdf.clone()), pdf)
    }

    fn app(db: sea_orm::DatabaseConnection, pdf: PdfRoot) -> Router {
        super::app(super::state(
            db,
            invoice::ares::DEFAULT_ARES_URL,
            invoice::cnb::DEFAULT_CNB_URL,
            pdf,
        ))
    }
}

/// GET with the test token; status, headers and raw body.
pub async fn get_raw(app: &Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = super::send(app.clone(), super::get(uri, Some(super::TEST_TOKEN))).await;
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 24)
        .await
        .expect("read body");
    (status, headers, bytes.to_vec())
}
