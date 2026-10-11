//! Login, sessions (expiry, logout, revoke-others, host binding), CSRF and
//! the login rate limits (auth.md, 4a).

mod common;

use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use common::auth::{As, exchange, login, on, request, session_cookie};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb, router};
use invoice::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::json;

fn invalid() -> (StatusCode, serde_json::Value) {
    (
        StatusCode::UNAUTHORIZED,
        json!({ "code": "invalid_credentials" }),
    )
}

#[tokio::test]
async fn login_sets_a_host_only_session_cookie() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = json!({ "email": OWNER_EMAIL, "password": OWNER_PASSWORD });
    let req = request(
        Method::POST,
        TEST_HOST,
        "/api/auth/login",
        As::Nobody,
        Some(body.clone()),
    )
    .header(header::USER_AGENT, "x".repeat(300))
    .body(Body::from(body.to_string()))
    .expect("request");
    let (status, headers, _) = exchange(&app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let set = headers[header::SET_COOKIE].to_str().expect("ascii");
    assert!(set.starts_with("invoice_session="), "{set}");
    assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains("Path=/"));
    assert!(!set.contains("Domain"), "host-only");
    assert!(!set.contains("Secure"), "http public URL");
    let cookie = session_cookie(&headers).expect("cookie");
    let (status, me) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["space"]["role"], "owner");
    let row = db
        .conn
        .query_one(sea_orm::Statement::from_string(
            db.conn.get_database_backend(),
            "SELECT length(user_agent) AS n, length(token_hash) AS h FROM sessions",
        ))
        .await
        .expect("query")
        .expect("one session");
    assert_eq!(row.try_get::<i32>("", "n").ok(), Some(200));
    assert_eq!(
        row.try_get::<i32>("", "h").ok(),
        Some(64),
        "sha256 stored, not the cookie"
    );
}

#[tokio::test]
async fn every_login_failure_is_the_same_401() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    assert_eq!(
        login(&app, TEST_HOST, OWNER_EMAIL, "wrong password!").await,
        Err(invalid())
    );
    assert_eq!(
        login(&app, TEST_HOST, "nobody@example.com", OWNER_PASSWORD).await,
        Err(invalid())
    );
    // A verified user of another space is not a member here.
    let (_, _) = common::auth::seed_space(&db.conn, "beta", "beta@example.com", None).await;
    assert_eq!(
        login(&app, TEST_HOST, "beta@example.com", OWNER_PASSWORD).await,
        Err(invalid())
    );
    assert!(
        login(
            &app,
            "beta.localhost:3000",
            "beta@example.com",
            OWNER_PASSWORD
        )
        .await
        .is_ok()
    );
    // Disabled.
    db.conn
        .execute_unprepared("UPDATE users SET disabled = true WHERE email = 'beta@example.com'")
        .await
        .expect("disable");
    assert_eq!(
        login(
            &app,
            "beta.localhost:3000",
            "beta@example.com",
            OWNER_PASSWORD
        )
        .await,
        Err(invalid())
    );
    assert_eq!(
        login(&app, BASE_HOST, "beta@example.com", OWNER_PASSWORD).await,
        Err(invalid())
    );
}

#[tokio::test]
async fn login_failures_are_rate_limited() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    for _ in 0..5 {
        assert_eq!(
            login(&app, TEST_HOST, OWNER_EMAIL, "wrong password!").await,
            Err(invalid())
        );
    }
    let body = json!({ "email": OWNER_EMAIL, "password": OWNER_PASSWORD });
    let req = request(
        Method::POST,
        TEST_HOST,
        "/api/auth/login",
        As::Nobody,
        Some(body.clone()),
    )
    .body(Body::from(body.to_string()))
    .expect("request");
    let (status, headers, json) = exchange(&app, req).await;
    assert_eq!(
        (status, json),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        )
    );
    let retry: u64 = headers[header::RETRY_AFTER]
        .to_str()
        .ok()
        .and_then(|v| v.parse().ok())
        .expect("Retry-After seconds");
    assert!((1..=900).contains(&retry), "{retry}");
    // Per e-mail: another account from the same client still logs in (IP: 20 / 15 min).
    common::auth::user(&db.conn, "other@example.com").await;
    assert!(
        login(&app, BASE_HOST, "other@example.com", OWNER_PASSWORD)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn sessions_expire_idle_and_absolute() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let me = |c: String| {
        let app = app.clone();
        async move {
            on(
                &app,
                Method::GET,
                TEST_HOST,
                "/api/auth/me",
                As::Cookie(&c),
                None,
            )
            .await
            .0
        }
    };
    let idle = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    db.conn
        .execute_unprepared(
            "UPDATE sessions SET last_seen_at = now() - interval '14 days 1 minute'",
        )
        .await
        .expect("idle");
    assert_eq!(me(idle).await, StatusCode::UNAUTHORIZED);
    let count = db
        .conn
        .query_one(sea_orm::Statement::from_string(
            db.conn.get_database_backend(),
            "SELECT count(*) AS n FROM sessions",
        ))
        .await
        .expect("count")
        .and_then(|r| r.try_get::<i64>("", "n").ok());
    assert_eq!(count, Some(0), "an expired session is deleted");

    let active = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    db.conn
        .execute_unprepared("UPDATE sessions SET last_seen_at = now() - interval '13 days'")
        .await
        .expect("still within idle");
    assert_eq!(me(active.clone()).await, StatusCode::OK);
    db.conn
        .execute_unprepared("UPDATE sessions SET expires_at = now() - interval '1 second'")
        .await
        .expect("absolute");
    assert_eq!(me(active).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_and_revoke_others() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let a = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("a");
    let b = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("b");
    let base = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("base");
    let status = |c: String, host: &'static str| {
        let app = app.clone();
        async move {
            on(
                &app,
                Method::GET,
                host,
                "/api/auth/me",
                As::Cookie(&c),
                None,
            )
            .await
            .0
        }
    };

    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/auth/sessions/revoke-others",
        As::Bearer(TEST_TOKEN),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "session only");
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/auth/sessions/revoke-others",
        As::Cookie(&a),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(status(a.clone(), TEST_HOST).await, StatusCode::OK);
    assert_eq!(status(b, TEST_HOST).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        status(base, BASE_HOST).await,
        StatusCode::UNAUTHORIZED,
        "all hosts"
    );
    assert_eq!(
        status(TEST_TOKEN.to_string(), TEST_HOST).await,
        StatusCode::UNAUTHORIZED,
        "a token is no cookie"
    );

    let req = request(
        Method::POST,
        TEST_HOST,
        "/api/auth/logout",
        As::Cookie(&a),
        None,
    )
    .body(Body::empty())
    .expect("request");
    let (s, headers, _) = exchange(&app, req).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(
        headers[header::SET_COOKIE]
            .to_str()
            .expect("ascii")
            .contains("Max-Age=0")
    );
    assert_eq!(status(a, TEST_HOST).await, StatusCode::UNAUTHORIZED);
    let bearer = |t: String| {
        let app = app.clone();
        async move {
            on(
                &app,
                Method::GET,
                TEST_HOST,
                "/api/auth/me",
                As::Bearer(&t),
                None,
            )
            .await
            .0
        }
    };
    // revoke-others took the user's API tokens; a logout does not.
    assert_eq!(bearer(TEST_TOKEN.into()).await, StatusCode::UNAUTHORIZED);
    let fresh = common::members::token(&db.conn, db.space, db.user_id, Role::Owner).await;
    let c = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let req = request(
        Method::POST,
        TEST_HOST,
        "/api/auth/logout",
        As::Cookie(&c),
        None,
    )
    .body(Body::empty())
    .expect("request");
    assert_eq!(exchange(&app, req).await.0, StatusCode::NO_CONTENT);
    assert_eq!(bearer(fresh).await, StatusCode::OK);
}
