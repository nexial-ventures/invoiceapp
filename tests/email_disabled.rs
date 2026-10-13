//! The kill switch `INVOICEAPP__EMAIL_DISABLED` (email.md): e-mails sent on
//! behalf of a space are refused with 503 `email_disabled`; account e-mails
//! (verify, reset, invitations) are skipped and logged.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, on, register_body, settle};
use common::documents::{create_issued, id};
use common::email_limits::{post, sends};
use common::members::count;
use common::smtp::{EmailEnv, history, issuable_with_emails, send_body};
use common::{BASE_HOST, TEST_TOKEN, TestDb};
use serde_json::json;

const OWNER: As<'static> = As::Bearer(TEST_TOKEN);

fn disabled(db: &TestDb, env: &EmailEnv, smtp: bool) -> axum::Router {
    let app = common::TestApp::builder(&db.conn)
        .mdcast(&env.pdf)
        .registration(true)
        .state(|s| s.email_disabled = true);
    if smtp { app.smtp(&env.config) } else { app }.build()
}

#[tokio::test]
async fn space_emails_are_refused() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let on_app = env.router(db.conn.clone());
    let doc = create_issued(&on_app, issuable_with_emails(&on_app).await).await;
    let uri = format!("/api/documents/{}/email", id(&doc));
    let code = json!({ "code": "email_disabled" });

    let app = disabled(&db, &env, true);
    // The UI learns it from the status and the prefill.
    let (_, status) = common::call(&app, Method::GET, "/api/settings/email", None).await;
    assert_eq!(
        (&status["configured"], &status["disabled"]),
        (&json!(true), &json!(true))
    );
    let (_, pre) = common::call(&app, Method::GET, &uri, None).await;
    assert_eq!(pre["disabled"], true, "{pre}");
    let (_, pre) = common::call(&on_app, Method::GET, &uri, None).await;
    assert_eq!(pre["disabled"], false, "{pre}");

    let (s, _, j) = post(&app, OWNER, &uri, send_body(json!({}))).await;
    assert_eq!((s, &j), (StatusCode::SERVICE_UNAVAILABLE, &code));
    let (s, _, j) = post(&app, OWNER, "/api/settings/email/test", json!({})).await;
    assert_eq!((s, &j), (StatusCode::SERVICE_UNAVAILABLE, &code));
    // The kill switch wins over a missing SMTP configuration.
    let no_smtp = disabled(&db, &env, false);
    let (s, _, j) = post(&no_smtp, OWNER, &uri, send_body(json!({}))).await;
    assert_eq!((s, &j), (StatusCode::SERVICE_UNAVAILABLE, &code));

    assert!(env.smtp.mails().is_empty());
    assert_eq!(
        history(&app, &id(&doc)).await.1,
        json!([]),
        "nothing logged"
    );
    assert_eq!(sends(&db.conn, None).await, 0, "nothing counted");
}

#[tokio::test]
async fn account_emails_are_skipped() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = disabled(&db, &env, true);

    // An invitation is created, but neither mailed nor its link returned.
    let invite = json!({ "email": "new@example.com", "role": "member" });
    let (s, _, j) = post(&app, OWNER, "/api/invites", invite).await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    assert_eq!(
        (&j["emailSent"], &j["emailDisabled"]),
        (&json!(false), &json!(true))
    );
    assert!(j.get("url").is_none(), "{j}");
    assert_eq!(
        count(&db.conn, "SELECT count(*) FROM space_invites").await,
        1
    );

    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body("jana@example.com")),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset",
        As::Nobody,
        Some(json!({ "email": common::OWNER_EMAIL })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);

    settle().await;
    assert!(env.smtp.mails().is_empty(), "no account e-mail sent");
    assert_eq!(sends(&db.conn, None).await, 0);
}
