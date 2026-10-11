//! Password change and reset by e-mail (auth.md, 4a).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, mail, mail_app, mail_token, on, settle};
use common::smtp::EmailEnv;
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb, router};
use sea_orm::ConnectionTrait;
use serde_json::json;

const NEW: &str = "a brand new password";

#[tokio::test]
async fn change_password_revokes_the_other_sessions() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let me = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let other = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("other");

    let body = |current: &str, new: &str| json!({ "currentPassword": current, "newPassword": new });
    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/account/password",
        As::Cookie(&me),
        Some(body("wrong password!", "short")),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        j["fields"],
        json!({ "currentPassword": "invalid", "newPassword": "too_short" })
    );
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/account/password",
        As::Bearer(TEST_TOKEN),
        Some(body(OWNER_PASSWORD, NEW)),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "session only");
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/account/password",
        As::Cookie(&me),
        Some(body(OWNER_PASSWORD, NEW)),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&me),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "the current session stays");
    let (s, _) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/auth/me",
        As::Cookie(&other),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Bearer(TEST_TOKEN),
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "the user's API tokens are revoked"
    );
    assert!(
        login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
            .await
            .is_err()
    );
    assert!(login(&app, TEST_HOST, OWNER_EMAIL, NEW).await.is_ok());
}

#[tokio::test]
async fn reset_by_mail_sets_the_password_verifies_and_revokes_sessions() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, false);
    let session = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    db.conn
        .execute_unprepared("UPDATE users SET email_verified_at = NULL")
        .await
        .expect("unverify");

    // Any known host; unknown e-mails get the same answer and no mail.
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/auth/password-reset",
        As::Nobody,
        Some(json!({ "email": "nobody@example.com" })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    settle().await;
    assert!(env.smtp.mails().is_empty());
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/auth/password-reset",
        As::Nobody,
        Some(json!({ "email": OWNER_EMAIL })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let token = mail_token(&mail(&env.smtp, 1).await, "reset");

    let confirm = |t: &str, p: &str| json!({ "token": t, "password": p });
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset/confirm",
        As::Nobody,
        Some(confirm(&token, "short")),
    )
    .await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "password": "too_short" })
        )
    );
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset/confirm",
        As::Nobody,
        Some(confirm(&token, NEW)),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset/confirm",
        As::Nobody,
        Some(confirm(&token, NEW)),
    )
    .await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "token": "invalid" })
        ),
        "single use"
    );

    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&session),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "every session revoked");
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Bearer(TEST_TOKEN),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "API tokens are revoked too");
    let fresh = login(&app, TEST_HOST, OWNER_EMAIL, NEW)
        .await
        .expect("new password");
    let (_, me) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&fresh),
        None,
    )
    .await;
    assert_eq!(me["user"]["emailVerified"], true);
}

#[tokio::test]
async fn expired_or_foreign_reset_tokens_are_invalid() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, true);
    on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset",
        As::Nobody,
        Some(json!({ "email": OWNER_EMAIL })),
    )
    .await;
    let token = mail_token(&mail(&env.smtp, 1).await, "reset");
    db.conn
        .execute_unprepared("UPDATE user_tokens SET expires_at = now() - interval '1 second'")
        .await
        .expect("expire");
    let body = json!({ "token": token, "password": NEW });
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset/confirm",
        As::Nobody,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    // A verification token is no reset token.
    let reg = json!({ "email": "new@example.com", "password": NEW, "displayName": "N" });
    on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(reg),
    )
    .await;
    let verify = mail_token(&mail(&env.smtp, 2).await, "verify");
    let body = json!({ "token": verify, "password": NEW });
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset/confirm",
        As::Nobody,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
            .await
            .is_ok(),
        "unchanged"
    );
}

#[tokio::test]
async fn reset_requests_are_rate_limited_per_email() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, false);
    for _ in 0..3 {
        let (s, _) = on(
            &app,
            Method::POST,
            TEST_HOST,
            "/api/auth/password-reset",
            As::Nobody,
            Some(json!({ "email": OWNER_EMAIL })),
        )
        .await;
        assert_eq!(s, StatusCode::ACCEPTED);
    }
    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/auth/password-reset",
        As::Nobody,
        Some(json!({ "email": OWNER_EMAIL })),
    )
    .await;
    assert_eq!(
        (s, j),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        )
    );
    mail(&env.smtp, 3).await;
    settle().await;
    assert_eq!(env.smtp.mails().len(), 3);
}
