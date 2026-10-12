//! The progressive per-e-mail login delay and the hard per-IP limit
//! (auth.md, rate limits; issue #40).

mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use common::auth::{As, exchange, login, request, session_cookie};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TestDb, router};
use serde_json::json;

const WRONG: &str = "wrong password!";

/// One login: `Ok(())` signed in, else the status and `Retry-After`.
async fn attempt(
    app: &Router,
    email: &str,
    password: &str,
) -> Result<(), (StatusCode, Option<u64>)> {
    let body = json!({ "email": email, "password": password });
    let req = request(
        Method::POST,
        TEST_HOST,
        "/api/auth/login",
        As::Nobody,
        Some(body.clone()),
    )
    .body(Body::from(body.to_string()))
    .expect("request");
    let (status, headers, _) = exchange(app, req).await;
    if status == StatusCode::NO_CONTENT && session_cookie(&headers).is_some() {
        return Ok(());
    }
    let retry = headers
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    Err((status, retry))
}

fn limited(secs: u64) -> Result<(), (StatusCode, Option<u64>)> {
    Err((StatusCode::TOO_MANY_REQUESTS, Some(secs)))
}

fn wrong() -> Result<(), (StatusCode, Option<u64>)> {
    Err((StatusCode::UNAUTHORIZED, None))
}

async fn wait(secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
}

#[tokio::test]
async fn the_delay_grows_and_a_correct_password_after_it_signs_in() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    for _ in 0..3 {
        assert_eq!(attempt(&app, OWNER_EMAIL, WRONG).await, wrong());
    }
    // Inside the delay even the correct password is refused (unchecked).
    assert_eq!(attempt(&app, OWNER_EMAIL, OWNER_PASSWORD).await, limited(1));
    wait(1).await;
    assert_eq!(attempt(&app, OWNER_EMAIL, WRONG).await, wrong());
    assert_eq!(attempt(&app, OWNER_EMAIL, OWNER_PASSWORD).await, limited(2));
    wait(2).await;
    assert_eq!(attempt(&app, OWNER_EMAIL, OWNER_PASSWORD).await, Ok(()));
}

#[tokio::test]
async fn a_delayed_e_mail_does_not_affect_another() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    common::auth::user(&db.conn, "other@example.com").await;
    for _ in 0..3 {
        assert_eq!(attempt(&app, OWNER_EMAIL, WRONG).await, wrong());
    }
    assert_eq!(attempt(&app, OWNER_EMAIL, WRONG).await, limited(1));
    // (Not a member of the test space: signs in on the base host.)
    assert!(
        login(&app, BASE_HOST, "other@example.com", OWNER_PASSWORD)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn the_ip_limit_stays_hard() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    // Twenty failures from one client, spread over e-mails (none delayed).
    for i in 0..20 {
        let email = format!("nobody{i}@example.com");
        assert_eq!(attempt(&app, &email, WRONG).await, wrong());
    }
    // The IP bucket refuses even a correct login of an untouched e-mail,
    // until the window frees up — no short delay here.
    match attempt(&app, OWNER_EMAIL, OWNER_PASSWORD).await {
        Err((StatusCode::TOO_MANY_REQUESTS, Some(s))) => assert!(s > 30 * 20, "{s}"),
        other => panic!("expected the hard IP limit, got {other:?}"),
    }
    assert!(
        login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
            .await
            .is_err()
    );
}
