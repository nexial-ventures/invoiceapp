//! Rate limits under concurrency and on password checks (auth.md, 4a).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, mail, mail_app, on, register_body, settle};
use common::smtp::EmailEnv;
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TestDb, router};
use sea_orm::ConnectionTrait;
use serde_json::json;

#[tokio::test]
async fn a_parallel_login_burst_cannot_pass_the_limit() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let tries: Vec<_> = (0..12)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(
                async move { login(&app, TEST_HOST, OWNER_EMAIL, "wrong password!").await },
            )
        })
        .collect();
    let mut statuses = Vec::new();
    for t in tries {
        statuses.push(t.await.expect("join").expect_err("wrong password").0);
    }
    let failed = statuses
        .iter()
        .filter(|s| **s == StatusCode::UNAUTHORIZED)
        .count();
    let limited = statuses
        .iter()
        .filter(|s| **s == StatusCode::TOO_MANY_REQUESTS)
        .count();
    // Three free failures; the fourth would have to wait 1 s after the third.
    assert_eq!((failed, limited), (3, 9), "{statuses:?}");
}

#[tokio::test]
async fn successful_logins_do_not_count() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    for _ in 0..8 {
        login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
            .await
            .expect("login");
    }
}

#[tokio::test]
async fn own_password_checks_are_limited() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let cookie = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let change =
        json!({ "currentPassword": "wrong password!", "newPassword": "a brand new password" });
    for _ in 0..2 {
        let (s, _) = on(
            &app,
            Method::POST,
            TEST_HOST,
            "/api/account/password",
            As::Cookie(&cookie),
            Some(change.clone()),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let del = json!({ "slug": "acme", "password": "wrong password!" });
    {
        let (s, _) = on(
            &app,
            Method::DELETE,
            TEST_HOST,
            "/api/space",
            As::Cookie(&cookie),
            Some(del.clone()),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    }
    // Three failures in the user's bucket (shared with login): now the delay runs.
    let (s, j) = on(
        &app,
        Method::DELETE,
        TEST_HOST,
        "/api/space",
        As::Cookie(&cookie),
        Some(del),
    )
    .await;
    assert_eq!(
        (s, j),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        )
    );
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/account/password",
        As::Cookie(&cookie),
        Some(change),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    let err = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect_err("the login bucket is in its delay too");
    assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn reset_confirm_is_limited_per_ip() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = json!({ "token": "guess", "password": "a brand new password" });
    for _ in 0..20 {
        let (s, _) = on(
            &app,
            Method::POST,
            BASE_HOST,
            "/api/auth/password-reset/confirm",
            As::Nobody,
            Some(body.clone()),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset/confirm",
        As::Nobody,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn disabled_accounts_get_no_mail() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, true);
    db.conn
        .execute_unprepared("UPDATE users SET disabled = true")
        .await
        .expect("disable");
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body(OWNER_EMAIL)),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset",
        As::Nobody,
        Some(json!({ "email": OWNER_EMAIL })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    settle().await;
    assert!(env.smtp.mails().is_empty());
    // A new account still gets its verification mail.
    on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body("new@example.com")),
    )
    .await;
    mail(&env.smtp, 1).await;
}
