//! Render limits (pdf.md, Render limits): one preview in flight and 60 per
//! hour per space, the instance-wide render slots, the preview timeout — and
//! issuing never refused by the preview limits.

mod common;

use std::time::Duration;

use axum::Router;
use axum::http::{StatusCode, header};
use common::TestDb;
use common::documents::{create_doc, id, issuable, issue};
use common::mdcast::{PdfEnv, get_raw};
use invoice::pdf::limits::{PREVIEW_TIMEOUT, QUEUE_WAIT, RenderLimits};
use serde_json::Value;

const PREVIEW: &str = "/api/pdf/preview";
const DRAFT_PREVIEW: &str = "/api/pdf/design/draft/preview";

fn spawn_get(app: &Router, uri: &str) -> tokio::task::JoinHandle<StatusCode> {
    let (app, uri) = (app.clone(), uri.to_string());
    tokio::spawn(async move { get_raw(&app, &uri).await.0 })
}

fn spawn_issue(app: &Router, doc: &Value) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let (app, doc_id) = (app.clone(), id(doc));
    tokio::spawn(async move { issue(&app, &doc_id).await })
}

fn code(body: &[u8]) -> Value {
    serde_json::from_slice(body).expect("JSON error body")
}

#[tokio::test]
async fn one_preview_in_flight_per_space_but_issue_passes() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let body = issuable(&app).await;
    let draft = create_doc(&app, body.clone()).await;
    let to_issue = create_doc(&app, body).await;

    env.mock.hold();
    let first = spawn_get(&app, PREVIEW);
    env.mock.wait_started(1).await;
    let draft_pdf = format!("/api/documents/{}/pdf", id(&draft));
    for uri in [PREVIEW, DRAFT_PREVIEW, draft_pdf.as_str()] {
        let (status, headers, body) = get_raw(&app, uri).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{uri}");
        assert_eq!(code(&body)["code"], "rate_limited");
        assert_eq!(headers[header::RETRY_AFTER], "1");
    }
    // Issuing is not a preview: it renders while the preview still runs.
    let issued = spawn_issue(&app, &to_issue);
    env.mock.wait_started(2).await;
    env.mock.release();
    assert_eq!(first.await.expect("preview task"), StatusCode::OK);
    let (status, doc) = issued.await.expect("issue task");
    assert_eq!(status, StatusCode::OK, "{doc}");
    // The slot is free again.
    assert_eq!(get_raw(&app, &draft_pdf).await.0, StatusCode::OK);
}

#[tokio::test]
async fn previews_per_hour() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    for n in 0..60 {
        let uri = if n % 2 == 0 { PREVIEW } else { DRAFT_PREVIEW };
        assert_eq!(get_raw(&app, uri).await.0, StatusCode::OK, "preview {n}");
    }
    let (status, headers, body) = get_raw(&app, PREVIEW).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(code(&body)["code"], "rate_limited");
    let retry: u64 = headers[header::RETRY_AFTER]
        .to_str()
        .expect("ascii")
        .parse()
        .expect("seconds");
    assert!(retry > 3500, "{retry}");
    assert_eq!(env.mock.count(), 60);
    // Issuing still works.
    let doc = create_doc(&app, issuable(&app).await).await;
    let (status, doc) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
}

#[tokio::test]
async fn full_render_slots_answer_busy() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let limits = RenderLimits::new(1).with_waits(Duration::from_millis(300), PREVIEW_TIMEOUT);
    let app = env.router_limited(db.conn.clone(), limits);
    let doc = create_doc(&app, issuable(&app).await).await;

    env.mock.hold();
    let issued = spawn_issue(&app, &doc);
    env.mock.wait_started(1).await;
    let (status, headers, body) = get_raw(&app, PREVIEW).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        code(&body),
        serde_json::json!({ "code": "pdf_unavailable", "detail": "busy" })
    );
    assert_eq!(headers[header::RETRY_AFTER], "10");
    env.mock.release();
    assert_eq!(issued.await.expect("issue task").0, StatusCode::OK);
    assert_eq!(get_raw(&app, PREVIEW).await.0, StatusCode::OK);
    assert_eq!(
        env.mock.count(),
        2,
        "the refused preview never reached mdcast"
    );
}

#[tokio::test]
async fn previews_time_out_sooner_than_issue() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let limits = RenderLimits::default().with_waits(QUEUE_WAIT, Duration::from_millis(300));
    let app = env.router_limited(db.conn.clone(), limits);
    let doc = create_doc(&app, issuable(&app).await).await;

    env.mock.delay(Duration::from_millis(1200));
    let (status, _, body) = get_raw(&app, PREVIEW).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        code(&body),
        serde_json::json!({ "code": "pdf_unavailable" })
    );
    let (status, doc) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
}
