//! Sign in with Google (google.md, 4d): linking needs a verified user and a
//! single-use grant; a link made while the address was unproven does not
//! survive its verification; the callback writes no account; internal
//! failures end in the `unavailable` redirect.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, on};
use common::google::{self, at, link_query, link_request, sign_in};
use common::members::{accept, count, invited};
use common::oidc::claims;
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TestDb};
use sea_orm::ConnectionTrait;
use serde_json::json;

async fn unverify(db: &TestDb, email: &str) {
    db.conn
        .execute_unprepared(&format!(
            "UPDATE users SET email_verified_at = NULL WHERE email = '{email}'"
        ))
        .await
        .expect("unverify");
}

#[tokio::test]
async fn link_grant_rules() {
    let db = TestDb::new().await;
    let off = common::router(db.conn.clone());
    let base = login(&off, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    assert_eq!(
        link_request(&off, BASE_HOST, &base, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (app, mock) = google::app(&db.conn, false);
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/account/google/link",
        As::Bearer(common::TEST_TOKEN),
        Some(json!({ "password": OWNER_PASSWORD })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "session only");
    let body = json!({ "password": "not the password" });
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/account/google/link",
        As::Cookie(&base),
        Some(body),
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "password": "invalid" })
        )
    );
    // Another user's grant: refused, and it stays usable by its owner.
    let q = link_query(&app, BASE_HOST, &base).await;
    common::auth::user(&db.conn, "second@example.com").await;
    let other = login(&app, BASE_HOST, "second@example.com", OWNER_PASSWORD)
        .await
        .expect("login");
    let cookie = vec![format!("invoice_session={other}")];
    let (_, h, _) = google::get(
        &app,
        BASE_HOST,
        &format!("/api/auth/google/start?{q}"),
        &cookie,
    )
    .await;
    assert_eq!(
        google::location(&h),
        at(BASE_HOST, "/account?google_error=invalid")
    );
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        &q,
        Some(&base),
        "sub-o",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(done.location, at(BASE_HOST, "/account?google=linked"));
    // Single use.
    let cookie = vec![format!("invoice_session={base}")];
    let (_, h, _) = google::get(
        &app,
        BASE_HOST,
        &format!("/api/auth/google/start?{q}"),
        &cookie,
    )
    .await;
    assert_eq!(
        google::location(&h),
        at(BASE_HOST, "/account?google_error=invalid")
    );
}

/// Someone registers the victim's address (never verified) and tries to
/// attach their own Google account to it.
#[tokio::test]
async fn unverified_users_cannot_link() {
    let db = TestDb::new().await;
    let (app, _mock) = google::app(&db.conn, false);
    common::auth::user(&db.conn, "victim@example.com").await;
    unverify(&db, "victim@example.com").await;
    let squat = login(&app, BASE_HOST, "victim@example.com", OWNER_PASSWORD)
        .await
        .expect("an unverified user logs in on the base host");
    let (s, j) = link_request(&app, BASE_HOST, &squat, None).await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "email_unverified" }))
    );
}

/// A Google account linked while the address was unproven (older data) is
/// dropped when the address gets verified — by the owner's Google login or
/// by any other proof (here: an invitation accept).
#[tokio::test]
async fn unproven_links_do_not_survive_verification() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let id = common::auth::user(&db.conn, "victim@example.com").await;
    unverify(&db, "victim@example.com").await;
    db.conn
        .execute_unprepared(
            "UPDATE users SET google_sub = 'sub-squat' WHERE email = 'victim@example.com'",
        )
        .await
        .expect("squatter link");
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-victim",
        "victim@example.com",
    )
    .await;
    let cookie = done.session.expect("the owner claims the account");
    let (_, me) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/auth/me",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(me["user"]["id"], id.to_string());
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-squat",
        "squatter@example.com",
    )
    .await;
    assert_eq!(
        done.location,
        at(BASE_HOST, "/login?google_error=not_allowed")
    );

    common::auth::user(&db.conn, "other@example.com").await;
    unverify(&db, "other@example.com").await;
    db.conn
        .execute_unprepared(
            "UPDATE users SET google_sub = 'sub-squat2' WHERE email = 'other@example.com'",
        )
        .await
        .expect("squatter link");
    let token = invited(&app, "other@example.com", "member").await;
    let body = json!({ "token": token, "password": OWNER_PASSWORD });
    let (s, _, j, _) = accept(&app, TEST_HOST, body).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    let linked = "SELECT count(*) FROM users WHERE email = 'other@example.com' \
                  AND google_sub IS NOT NULL";
    assert_eq!(count(&db.conn, linked).await, 0);
}

/// Accounts are written only by `finish`, after the browser binding.
#[tokio::test]
async fn callback_writes_no_account() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, true);
    common::auth::user(&db.conn, "claim@example.com").await;
    unverify(&db, "claim@example.com").await;
    for (sub, email) in [
        ("sub-new", "new@example.com"),
        ("sub-claim", "claim@example.com"),
    ] {
        let st = google::start(&app, &mock, BASE_HOST, "purpose=login", &[]).await;
        let to = google::callback(&app, &mock, &st, claims(&mock, sub, email, &st.nonce)).await;
        let done = google::finish(&app, &to, BASE_HOST, &[]).await;
        assert_eq!(done.location, at(BASE_HOST, "/login?google_error=invalid"));
    }
    assert_eq!(
        count(
            &db.conn,
            "SELECT count(*) FROM users WHERE email = 'new@example.com'"
        )
        .await,
        0
    );
    let untouched = "SELECT count(*) FROM users WHERE email = 'claim@example.com' \
                     AND password_hash IS NOT NULL AND google_sub IS NULL AND email_verified_at IS NULL";
    assert_eq!(count(&db.conn, untouched).await, 1);
}

#[tokio::test]
async fn internal_failures_redirect_as_unavailable() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    db.conn
        .execute_unprepared("DROP TABLE google_handoffs")
        .await
        .expect("break the handoffs");
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    assert_eq!(to, at(TEST_HOST, "/login?google_error=unavailable"));
    let done = google::finish(
        &app,
        &at(TEST_HOST, "/api/auth/google/finish?token=x"),
        TEST_HOST,
        &[],
    )
    .await;
    assert_eq!(
        done.location,
        at(TEST_HOST, "/login?google_error=unavailable")
    );
    db.conn
        .execute_unprepared("DROP TABLE google_flows")
        .await
        .expect("break the flows");
    let (s, h, _) = google::get(&app, TEST_HOST, "/api/auth/google/start?purpose=login", &[]).await;
    assert_eq!(
        (s, google::location(&h)),
        (
            StatusCode::FOUND,
            at(TEST_HOST, "/login?google_error=unavailable")
        )
    );
    assert_eq!(
        google::callback_raw(&app, "state=x&code=y").await,
        at(BASE_HOST, "/login?google_error=unavailable")
    );
}
