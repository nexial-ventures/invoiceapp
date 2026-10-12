//! Accepting an invitation: new account (registration off), existing
//! account, wrong password, used / expired token, already a member, CSRF
//! (members.md, 4b).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, on, user};
use common::members::{accept, count, invited, join, show};
use common::{BASE_HOST, OWNER_PASSWORD, TEST_HOST, TestDb, router};
use invoice::space::Role;
use serde_json::{Value, json};

const NEW_PASSWORD: &str = "a brand new password";

async fn me(app: &axum::Router, cookie: &str) -> Value {
    let (s, me) = on(
        app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{me}");
    me
}

#[tokio::test]
async fn a_new_account_is_created_verified_while_registration_is_off() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let token = invited(&app, "nova@example.com", "member").await;

    let (s, info) = show(&app, &token).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        info,
        json!({ "space": { "slug": "acme", "name": "Space acme" }, "email": "nova@example.com",
                "role": "member", "accountExists": false, "requireMfa": false })
    );

    let (s, cookie, j, _) = accept(
        &app,
        TEST_HOST,
        json!({ "token": token, "password": "short" }),
    )
    .await;
    assert_eq!((s, cookie), (StatusCode::UNPROCESSABLE_ENTITY, None));
    assert_eq!(
        j["fields"],
        json!({ "displayName": "required", "password": "too_short" })
    );
    let body =
        json!({ "token": token, "password": NEW_PASSWORD, "displayName": " Nová Nováková " });
    let (s, cookie, j, _) = accept(&app, TEST_HOST, body.clone()).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    let cookie = cookie.expect("session cookie");
    let me = me(&app, &cookie).await;
    assert_eq!(me["user"]["email"], "nova@example.com");
    assert_eq!(me["user"]["displayName"], "Nová Nováková");
    assert_eq!(me["user"]["emailVerified"], true);
    assert_eq!(me["space"]["role"], "member");

    // Single use; the account logs in with its password like any other.
    let (s, _, j, _) = accept(&app, TEST_HOST, body).await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "token": "invalid" })
        )
    );
    assert_eq!(show(&app, &token).await.0, StatusCode::NOT_FOUND);
    common::auth::login(&app, TEST_HOST, "nova@example.com", NEW_PASSWORD)
        .await
        .expect("login");
    assert_eq!(
        count(&db.conn, "SELECT count(*) FROM space_invites").await,
        0
    );
}

#[tokio::test]
async fn an_existing_account_accepts_with_its_password() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let jan = user(&db.conn, "jan@example.com").await;
    let token = invited(&app, "Jan@Example.com", "accountant").await;
    let (_, info) = show(&app, &token).await;
    assert_eq!(info["accountExists"], true);

    let wrong = json!({ "token": token, "password": "not the password" });
    let (s, cookie, j, _) = accept(&app, TEST_HOST, wrong).await;
    assert_eq!(
        (s, cookie, j),
        (
            StatusCode::UNAUTHORIZED,
            None,
            json!({ "code": "invalid_credentials" })
        )
    );
    let body = json!({ "token": token, "password": OWNER_PASSWORD, "displayName": "Ignored" });
    let (s, cookie, _, _) = accept(&app, TEST_HOST, body).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let me = me(&app, &cookie.expect("cookie")).await;
    assert_eq!(
        (me["user"]["id"].clone(), me["user"]["displayName"].clone()),
        (json!(jan), json!("Test User"))
    );
    assert_eq!(me["space"]["role"], "accountant");
}

#[tokio::test]
async fn wrong_passwords_count_in_the_login_bucket() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    user(&db.conn, "jan@example.com").await;
    let token = invited(&app, "jan@example.com", "member").await;
    let wrong = json!({ "token": token, "password": "not the password" });
    for _ in 0..3 {
        assert_eq!(
            accept(&app, TEST_HOST, wrong.clone()).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (s, _, j, headers) = accept(&app, TEST_HOST, wrong).await;
    assert_eq!(
        (s, j),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        )
    );
    assert!(headers.contains_key("retry-after"));
    // The same bucket as login.
    let err = common::auth::login(&app, TEST_HOST, "jan@example.com", OWNER_PASSWORD).await;
    assert_eq!(err.map_err(|e| e.0), Err(StatusCode::TOO_MANY_REQUESTS));
}

#[tokio::test]
async fn a_disabled_account_is_refused() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    user(&db.conn, "jan@example.com").await;
    sea_orm::ConnectionTrait::execute_unprepared(
        &db.conn,
        "UPDATE users SET disabled = true WHERE email = 'jan@example.com'",
    )
    .await
    .expect("disable");
    let token = invited(&app, "jan@example.com", "member").await;
    let body = json!({ "token": token, "password": OWNER_PASSWORD });
    let (s, _, j, _) = accept(&app, TEST_HOST, body).await;
    assert_eq!(
        (s, j),
        (
            StatusCode::UNAUTHORIZED,
            json!({ "code": "invalid_credentials" })
        )
    );
    assert_eq!(show(&app, &token).await.0, StatusCode::OK, "not consumed");
}

#[tokio::test]
async fn already_a_member_meanwhile_keeps_the_role() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let jan = user(&db.conn, "jan@example.com").await;
    let token = invited(&app, "jan@example.com", "admin").await;
    join(&db.conn, db.space, jan, Role::Accountant).await;
    let body = json!({ "token": token, "password": OWNER_PASSWORD });
    let (s, cookie, _, _) = accept(&app, TEST_HOST, body).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        me(&app, &cookie.expect("cookie")).await["space"]["role"],
        "accountant"
    );
    assert_eq!(
        show(&app, &token).await.0,
        StatusCode::NOT_FOUND,
        "consumed"
    );
}

#[tokio::test]
async fn invalid_expired_and_foreign_tokens() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = |t: &str| json!({ "token": t, "password": NEW_PASSWORD, "displayName": "Nova" });
    let (s, _, j, _) = accept(&app, TEST_HOST, body("garbage")).await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "token": "invalid" })
        )
    );
    assert_eq!(show(&app, "").await.0, StatusCode::NOT_FOUND);

    let token = invited(&app, "nova@example.com", "member").await;
    // Not on the base host.
    assert_eq!(
        accept(&app, BASE_HOST, body(&token)).await.0,
        StatusCode::NOT_FOUND
    );
    // A cross-site post is refused before anything else.
    let req = common::auth::request(
        Method::POST,
        TEST_HOST,
        "/api/invites/accept",
        As::Nobody,
        Some(body(&token)),
    )
    .header("origin", "http://evil.example.com")
    .body(axum::body::Body::from(body(&token).to_string()))
    .expect("request");
    let (s, _, j) = common::auth::exchange(&app, req).await;
    assert_eq!((s, j), (StatusCode::FORBIDDEN, json!({ "code": "csrf" })));

    sea_orm::ConnectionTrait::execute_unprepared(
        &db.conn,
        "UPDATE space_invites SET expires_at = now() - interval '1 second'",
    )
    .await
    .expect("expire");
    let (s, _, j, _) = accept(&app, TEST_HOST, body(&token)).await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "token": "invalid" })
        )
    );
    assert_eq!(show(&app, &token).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        count(
            &db.conn,
            "SELECT count(*) FROM users WHERE email = 'nova@example.com'"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn accepting_ends_the_session_of_another_user_on_this_host() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let old = common::members::session_on(&db.conn, db.space, db.user_id).await;
    let token = invited(&app, "nova@example.com", "member").await;
    let body = json!({ "token": token, "password": NEW_PASSWORD, "displayName": "Nova" });
    let req = common::auth::request(
        Method::POST,
        TEST_HOST,
        "/api/invites/accept",
        As::Cookie(&old),
        Some(body.clone()),
    )
    .body(axum::body::Body::from(body.to_string()))
    .expect("request");
    let (s, headers, _) = common::auth::exchange(&app, req).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let new = common::auth::session_cookie(&headers).expect("new cookie");
    assert_eq!(me(&app, &new).await["user"]["email"], "nova@example.com");
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&old),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "the previous session is gone");
}

#[tokio::test]
async fn token_lookups_are_rate_limited_per_ip() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    for _ in 0..20 {
        assert_eq!(show(&app, "guess").await.0, StatusCode::NOT_FOUND);
    }
    let (s, j) = show(&app, "guess").await;
    assert_eq!(
        (s, j),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        )
    );
    let (s, _, _, _) = accept(&app, TEST_HOST, json!({ "token": "guess" })).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "shared with accept");
}
