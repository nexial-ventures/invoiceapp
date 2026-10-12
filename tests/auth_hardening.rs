//! Security hardening (auth.md, mfa.md, spaces.md): an unverified account
//! cannot set up credentials, verification drops a second factor set up
//! before it, password events revoke API tokens, a token cannot mint one
//! that outlives it.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, on, seed_space};
use common::members::{count, join, token};
use common::mfa::enrol;
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb, router};
use invoice::auth::users::{self, TokenKind};
use invoice::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

async fn unverify(db: &TestDb) {
    db.conn
        .execute_unprepared("UPDATE users SET email_verified_at = NULL")
        .await
        .expect("unverify");
}

#[tokio::test]
async fn an_unverified_account_cannot_set_up_mfa_or_change_its_password() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    unverify(&db).await;
    let cookie = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("an unverified user may log in on the base host");
    let calls = [
        (
            "/api/account/mfa/setup",
            json!({ "password": OWNER_PASSWORD }),
        ),
        ("/api/account/mfa/enable", json!({ "code": "123456" })),
        (
            "/api/account/password",
            json!({ "currentPassword": OWNER_PASSWORD, "newPassword": "a brand new password" }),
        ),
    ];
    for (uri, body) in calls {
        let (s, j) = on(
            &app,
            Method::POST,
            BASE_HOST,
            uri,
            As::Cookie(&cookie),
            Some(body),
        )
        .await;
        assert_eq!(
            (s, j),
            (StatusCode::FORBIDDEN, json!({ "code": "email_unverified" })),
            "{uri}"
        );
    }
}

/// TOTP on, then the address becomes unproven again (as if the account had
/// been set up by whoever registered it).
async fn mfa_while_unverified(db: &TestDb, app: &axum::Router) {
    enrol(app, OWNER_EMAIL).await;
    db.conn
        .execute_unprepared(
            "UPDATE users SET totp_pending = 'x'::bytea, \
             totp_pending_expires_at = now() + interval '5 minutes'",
        )
        .await
        .expect("pending setup");
    unverify(db).await;
}

async fn second_factor_left(db: &TestDb) -> i64 {
    count(
        &db.conn,
        "SELECT (SELECT count(*) FROM users WHERE totp_secret IS NOT NULL \
         OR totp_pending IS NOT NULL OR totp_last_step IS NOT NULL) \
         + (SELECT count(*) FROM recovery_codes)",
    )
    .await
}

#[tokio::test]
async fn the_verify_link_drops_a_second_factor_set_up_while_unverified() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    mfa_while_unverified(&db, &app).await;
    assert!(second_factor_left(&db).await > 0);
    let raw = users::issue_token(&db.conn, db.user_id, TokenKind::Verify)
        .await
        .expect("token");
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify",
        As::Nobody,
        Some(json!({ "token": raw })),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    assert_eq!(second_factor_left(&db).await, 0);
    // A password alone signs in now (no code step).
    login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("no TOTP any more");
}

#[tokio::test]
async fn a_password_reset_drops_a_second_factor_set_up_while_unverified() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    mfa_while_unverified(&db, &app).await;
    let raw = users::issue_token(&db.conn, db.user_id, TokenKind::Reset)
        .await
        .expect("token");
    let body = json!({ "token": raw, "password": "a brand new password" });
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset/confirm",
        As::Nobody,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    assert_eq!(second_factor_left(&db).await, 0);
}

#[tokio::test]
async fn verifying_a_verified_account_keeps_its_second_factor() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    enrol(&app, OWNER_EMAIL).await;
    users::mark_verified(&db.conn, db.user_id)
        .await
        .expect("no-op");
    assert_eq!(
        count(
            &db.conn,
            "SELECT count(*) FROM users WHERE totp_secret IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(
        count(&db.conn, "SELECT count(*) FROM recovery_codes").await,
        10
    );
}

async fn me(app: &axum::Router, host: &str, bearer: &str) -> StatusCode {
    on(
        app,
        Method::GET,
        host,
        "/api/auth/me",
        As::Bearer(bearer),
        None,
    )
    .await
    .0
}

#[tokio::test]
async fn revoking_other_sessions_revokes_the_users_api_tokens_in_every_space() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", None).await;
    join(&db.conn, beta, db.user_id, Role::Admin).await;
    let in_beta = token(&db.conn, beta, db.user_id, Role::Admin).await;
    let someone_else =
        common::auth::member(&db.conn, db.space, "m@example.com", Role::Member).await;
    let beta_host = common::members::B_HOST;
    assert_eq!(me(&app, beta_host, &in_beta).await, StatusCode::OK);

    let cookie = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/auth/sessions/revoke-others",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        me(&app, TEST_HOST, TEST_TOKEN).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        me(&app, beta_host, &in_beta).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        me(&app, TEST_HOST, &someone_else).await,
        StatusCode::OK,
        "only the caller's tokens"
    );
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "the current session stays");
}

async fn mint(app: &axum::Router, bearer: &str, expires: Value) -> (StatusCode, Value) {
    let body = json!({ "name": "child", "role": "member", "expiresAt": expires });
    on(
        app,
        Method::POST,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(bearer),
        Some(body),
    )
    .await
}

#[tokio::test]
async fn a_token_cannot_mint_a_token_that_outlives_it() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let today = chrono::Utc::now().date_naive();
    let day = |n: i64| (today + chrono::Duration::days(n)).to_string();

    // TEST_TOKEN has no expiry: no cap.
    let (s, parent) = mint(&app, TEST_TOKEN, json!(day(5))).await;
    assert_eq!(s, StatusCode::CREATED, "{parent}");
    let (s, j) = mint(&app, TEST_TOKEN, Value::Null).await;
    assert_eq!(s, StatusCode::CREATED, "{j}");

    let parent = parent["token"].as_str().expect("token");
    for late in [json!(day(6)), Value::Null] {
        let (s, j) = mint(&app, parent, late.clone()).await;
        assert_eq!(
            (s, &j["fields"]),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                &json!({ "expiresAt": "too_late" })
            ),
            "{late}"
        );
    }
    for ok in [day(5), day(1)] {
        let (s, j) = mint(&app, parent, json!(ok)).await;
        assert_eq!(s, StatusCode::CREATED, "{ok}: {j}");
    }
    // A session has no cap.
    let cookie = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let body = json!({ "name": "s", "role": "member", "expiresAt": null });
    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/tokens",
        As::Cookie(&cookie),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
}

#[tokio::test]
async fn an_invite_accept_proved_by_totp_keeps_the_totp() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let email = "legacy@example.com";
    let uid = common::auth::user(&db.conn, email).await;
    let enrolled = enrol(&app, email).await;
    // A legacy account: TOTP set up while the address was unproven.
    let sql = format!("UPDATE users SET email_verified_at = NULL WHERE id = '{uid}'");
    db.conn.execute_unprepared(&sql).await.expect("unverify");
    common::mfa::require_mfa(&db.conn, common::TEST_SLUG).await;
    let token = common::members::invited(&app, email, "member").await;
    let code = common::mfa::fresh(&db.conn, uid, &enrolled.secret).await;
    let body = json!({ "token": token, "password": OWNER_PASSWORD, "code": code });
    let (s, cookie, j, _) = common::members::accept(&app, TEST_HOST, body).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    assert!(cookie.is_some());
    let sql = format!(
        "SELECT count(*) FROM users WHERE id = '{uid}' AND totp_secret IS NOT NULL \
         AND email_verified_at IS NOT NULL"
    );
    assert_eq!(count(&db.conn, &sql).await, 1, "verified, TOTP kept");
    let sql = format!("SELECT count(*) FROM recovery_codes WHERE user_id = '{uid}'");
    assert_eq!(count(&db.conn, &sql).await, 10);
}
