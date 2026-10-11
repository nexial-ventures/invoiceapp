//! The space storage quota under concurrency, and the design bytes of a
//! space that has none recorded yet (spaces.md, "Limits").

mod common;

use axum::http::StatusCode;
use common::TestDb;
use common::design::upload;
use common::documents::{create_contact, id, set_company};
use common::limits::{KIB, MIB, pdf_of, set_quota, used};
use common::mdcast::PdfEnv;
use common::received::{create_received, upload as upload_original};
use sea_orm::ConnectionTrait;
use serde_json::json;

/// Two received documents (no PDF yet).
async fn two_documents(app: &axum::Router) -> (String, String) {
    set_company(app, true).await;
    let s = create_contact(
        app,
        json!({ "name": "Dodavatel s.r.o.", "ico": "27074358" }),
    )
    .await;
    let a = id(&create_received(app, &s, json!({})).await);
    let b = id(&create_received(app, &s, json!({})).await);
    (a, b)
}

/// Exactly one of the two answers is the quota refusal, the other a success.
fn one_refused(statuses: [StatusCode; 2], ok: StatusCode) {
    let refused = statuses
        .iter()
        .filter(|s| **s == StatusCode::UNPROCESSABLE_ENTITY)
        .count();
    let passed = statuses.iter().filter(|s| **s == ok).count();
    assert_eq!((refused, passed), (1, 1), "{statuses:?}");
}

#[tokio::test]
async fn concurrent_original_uploads_cannot_share_the_remaining_space() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let (a, b) = two_documents(&app).await;
    set_quota(&db.conn, db.space, MIB).await;
    for _ in 0..3 {
        let (pa, pb) = (pdf_of(600 * KIB), pdf_of(600 * KIB));
        let ((sa, _), (sb, _)) = tokio::join!(
            upload_original(&app, &a, &pa),
            upload_original(&app, &b, &pb)
        );
        one_refused([sa, sb], StatusCode::NO_CONTENT);
        assert_eq!(used(&app).await, 600 * KIB as u64);
        // Free it again for the next round.
        for doc in [&a, &b] {
            let uri = format!("/api/documents/{doc}/original");
            common::call(&app, axum::http::Method::DELETE, &uri, None).await;
        }
        assert_eq!(used(&app).await, 0);
    }
}

#[tokio::test]
async fn concurrent_design_and_original_uploads_cannot_share_it_either() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let (a, _) = two_documents(&app).await;
    set_quota(&db.conn, db.space, MIB).await;
    let original = pdf_of(600 * KIB);
    let ((sd, _), (so, _)) = tokio::join!(
        upload(&app, "a.png", pdf_of(600 * KIB)),
        upload_original(&app, &a, &original)
    );
    let refused = [sd, so]
        .iter()
        .filter(|s| **s == StatusCode::UNPROCESSABLE_ENTITY)
        .count();
    assert_eq!(refused, 1, "design {sd}, original {so}");
    assert!(sd == StatusCode::OK || so == StatusCode::NO_CONTENT);
    assert_eq!(used(&app).await, 600 * KIB as u64);
}

#[tokio::test]
async fn design_bytes_are_recorded_for_a_space_without_a_usage_row() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let (a, _) = two_documents(&app).await;
    let (status, _) = upload(&app, "a.png", pdf_of(700 * KIB)).await;
    assert_eq!(status, StatusCode::OK);
    // As a space from before the limits migration: no usage row.
    let forget = format!(
        "DELETE FROM space_storage_usage WHERE space_id = '{}'",
        db.space
    );
    db.conn.execute_unprepared(&forget).await.expect("forget");
    assert_eq!(
        used(&app).await,
        700 * KIB as u64,
        "GET /api/space lists once"
    );

    db.conn.execute_unprepared(&forget).await.expect("forget");
    set_quota(&db.conn, db.space, MIB).await;
    let (status, body) = upload_original(&app, &a, &pdf_of(400 * KIB)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["detail"], "0.7 / 1 MB", "the first check lists once");
}
