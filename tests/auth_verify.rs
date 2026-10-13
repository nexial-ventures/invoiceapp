//! E-mail verification, resend and the mail rate limits (auth.md, 4a).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, PASSWORD, login, mail, mail_app, mail_token, on, register_body, settle};
use common::smtp::EmailEnv;
use common::{BASE_HOST, TestDb, router};
use sea_orm::ActiveModelTrait;
use serde_json::json;

#[tokio::test]
async fn unverified_user_cannot_log_in_on_a_space_host_and_resend_works() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, true);
    on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body("late@example.com")),
    )
    .await;
    let first = mail_token(&mail(&env.smtp, 1).await, "verify");

    // Even a membership would not help an unverified user on a space host.
    let user = invoiceapp::auth::users::find_by_email(&db.conn, "late@example.com")
        .await
        .expect("query")
        .expect("user");
    invoiceapp::space::entity::member::ActiveModel {
        space_id: sea_orm::Set(db.space.uuid()),
        user_id: sea_orm::Set(user.id),
        role: sea_orm::Set("member".into()),
        created_at: sea_orm::Set(chrono::Utc::now().into()),
    }
    .insert(&db.conn)
    .await
    .expect("membership");
    let err = login(&app, common::TEST_HOST, "late@example.com", PASSWORD)
        .await
        .expect_err("a member, but unverified");
    assert_eq!(
        err,
        (
            StatusCode::UNAUTHORIZED,
            json!({ "code": "invalid_credentials" })
        )
    );

    let (status, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify/resend",
        As::Nobody,
        Some(json!({ "email": "late@example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let second = mail_token(&mail(&env.smtp, 2).await, "verify");
    assert_ne!(first, second);
    let sent = env.smtp.mails().len();
    let (status, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify/resend",
        As::Nobody,
        Some(json!({ "email": "nobody@example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "unknown e-mail, same answer");
    settle().await;
    assert_eq!(env.smtp.mails().len(), sent, "and no mail");
    let (status, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify",
        As::Nobody,
        Some(json!({ "token": second })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify",
        As::Nobody,
        Some(json!({ "token": "made-up" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn expired_verification_token_is_invalid() {
    use sea_orm::ConnectionTrait;
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, true);
    on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body("old@example.com")),
    )
    .await;
    let token = mail_token(&mail(&env.smtp, 1).await, "verify");
    db.conn
        .execute_unprepared("UPDATE user_tokens SET expires_at = now() - interval '1 second'")
        .await
        .expect("age the token");
    let (status, body) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify",
        As::Nobody,
        Some(json!({ "token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"], json!({ "token": "invalid" }));
}

#[tokio::test]
async fn register_and_resend_are_rate_limited() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, true);
    for _ in 0..3 {
        let (status, _) = on(
            &app,
            Method::POST,
            BASE_HOST,
            "/api/auth/verify/resend",
            As::Nobody,
            Some(json!({ "email": "x@example.com" })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }
    let (status, body) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify/resend",
        As::Nobody,
        Some(json!({ "email": "x@example.com" })),
    )
    .await;
    assert_eq!(
        (status, body),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        ),
        "3 / hour per e-mail"
    );
    for i in 0..2 {
        let (status, _) = on(
            &app,
            Method::POST,
            BASE_HOST,
            "/api/auth/register",
            As::Nobody,
            Some(register_body(&format!("u{i}@example.com"))),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }
    let (status, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body("u9@example.com")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "5 / hour per IP across the mail routes"
    );
    // The plain router (no SMTP, registration off) is a separate instance.
    let (status, _) = on(
        &router(db.conn.clone()),
        Method::POST,
        BASE_HOST,
        "/api/auth/verify/resend",
        As::Nobody,
        Some(json!({ "email": "x@example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "limits are per instance");
}
