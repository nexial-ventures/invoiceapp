//! The progressive login delay per e-mail and client, the per-e-mail
//! ceiling and the hard per-IP limit (auth.md, rate limits; issue #40).

mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use common::auth::{As, exchange, request, session_cookie};
use common::{OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TestDb};
use invoice::ares::DEFAULT_ARES_URL;
use invoice::cnb::DEFAULT_CNB_URL;
use serde_json::json;

const WRONG: &str = "wrong password!";
const OWNER_IP: &str = "203.0.113.9";
const ATTACKER_IP: &str = "198.51.100.7";

type Outcome = Result<(), (StatusCode, Option<u64>)>;

/// The test app behind a "proxy": the client IP is the `X-Forwarded-For`
/// each attempt sends.
fn app(db: sea_orm::DatabaseConnection) -> Router {
    let (mdcast, _) = common::mdcast::spawn();
    let pdf = common::pdf_service(&mdcast, common::shared_storage());
    let mut state = common::state(db, DEFAULT_ARES_URL, DEFAULT_CNB_URL, pdf);
    state.trust_forwarded = true;
    common::app(state)
}

/// One login from `ip`: `Ok(())` signed in, else the status and `Retry-After`.
async fn attempt(app: &Router, ip: &str, email: &str, password: &str) -> Outcome {
    let body = json!({ "email": email, "password": password });
    let req = request(
        Method::POST,
        TEST_HOST,
        "/api/auth/login",
        As::Nobody,
        Some(body.clone()),
    )
    .header("x-forwarded-for", ip)
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

fn wrong() -> Outcome {
    Err((StatusCode::UNAUTHORIZED, None))
}

/// The correct password tried inside a delay of `secs`: 429 with at most
/// that wait — or, on a machine slow enough that the delay already ran out,
/// a sign-in (which changes nothing: a success is refunded).
fn delayed_at_most(outcome: Outcome, secs: u64) {
    match outcome {
        Err((StatusCode::TOO_MANY_REQUESTS, Some(s))) => assert!((1..=secs).contains(&s), "{s}"),
        Ok(()) => {}
        other => panic!("expected a delay of ≤ {secs} s, got {other:?}"),
    }
}

async fn wait(secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
}

#[tokio::test]
async fn the_delay_grows_and_a_correct_password_after_it_signs_in() {
    let db = TestDb::new().await;
    let app = app(db.conn.clone());
    for _ in 0..3 {
        assert_eq!(attempt(&app, OWNER_IP, OWNER_EMAIL, WRONG).await, wrong());
    }
    // Inside the delay even the correct password is refused (unchecked).
    delayed_at_most(
        attempt(&app, OWNER_IP, OWNER_EMAIL, OWNER_PASSWORD).await,
        1,
    );
    wait(1).await;
    assert_eq!(attempt(&app, OWNER_IP, OWNER_EMAIL, WRONG).await, wrong());
    delayed_at_most(
        attempt(&app, OWNER_IP, OWNER_EMAIL, OWNER_PASSWORD).await,
        2,
    );
    wait(2).await;
    assert_eq!(
        attempt(&app, OWNER_IP, OWNER_EMAIL, OWNER_PASSWORD).await,
        Ok(())
    );
}

#[tokio::test]
async fn an_attacker_only_delays_their_own_network() {
    let db = TestDb::new().await;
    let app = app(db.conn.clone());
    for _ in 0..3 {
        assert_eq!(
            attempt(&app, ATTACKER_IP, OWNER_EMAIL, WRONG).await,
            wrong()
        );
    }
    delayed_at_most(attempt(&app, ATTACKER_IP, OWNER_EMAIL, WRONG).await, 1);
    // The owner, from elsewhere, is not delayed — not even for a typo.
    assert_eq!(attempt(&app, OWNER_IP, OWNER_EMAIL, WRONG).await, wrong());
    assert_eq!(
        attempt(&app, OWNER_IP, OWNER_EMAIL, OWNER_PASSWORD).await,
        Ok(())
    );
    // Another e-mail from the attacker's network is not delayed either.
    common::auth::user(&db.conn, "other@example.com").await;
    assert_eq!(
        attempt(&app, ATTACKER_IP, "other@example.com", WRONG).await,
        wrong()
    );
}

#[tokio::test]
async fn the_e_mail_ceiling_stops_a_distributed_guess() {
    let db = TestDb::new().await;
    let app = app(db.conn.clone());
    // One failure each from 100 networks: no delay anywhere, no IP limit.
    for i in 0..100u32 {
        let ip = format!("10.0.{}.{}", i / 256, i % 256);
        assert_eq!(attempt(&app, &ip, OWNER_EMAIL, WRONG).await, wrong(), "{i}");
    }
    match attempt(&app, OWNER_IP, OWNER_EMAIL, OWNER_PASSWORD).await {
        Err((StatusCode::TOO_MANY_REQUESTS, Some(s))) => assert!(s > 30, "{s}"),
        other => panic!("expected the per-e-mail ceiling, got {other:?}"),
    }
}

#[tokio::test]
async fn the_ip_limit_stays_hard() {
    let db = TestDb::new().await;
    let app = app(db.conn.clone());
    // Twenty failures from one client, spread over e-mails (none delayed).
    for i in 0..20 {
        let email = format!("nobody{i}@example.com");
        assert_eq!(attempt(&app, ATTACKER_IP, &email, WRONG).await, wrong());
    }
    // The IP bucket refuses even a correct login of an untouched e-mail,
    // until the window frees up — no short delay here.
    match attempt(&app, ATTACKER_IP, OWNER_EMAIL, OWNER_PASSWORD).await {
        Err((StatusCode::TOO_MANY_REQUESTS, Some(s))) => assert!(s > 30, "{s}"),
        other => panic!("expected the hard IP limit, got {other:?}"),
    }
    // The same account from another network signs in.
    assert_eq!(
        attempt(&app, OWNER_IP, OWNER_EMAIL, OWNER_PASSWORD).await,
        Ok(())
    );
}
