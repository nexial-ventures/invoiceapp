//! Sign in with Google (google.md, 4d): accounts without a password —
//! password login refused, setting one within 10 minutes of a Google login,
//! `password: not_set` on password-requiring actions, unlinking.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, on};
use common::google::{self, sign_in};
use common::members::invited;
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TestDb};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

const EMAIL: &str = "gonly@example.com";
const NEW_PASSWORD: &str = "a brand new password";

fn not_set(field: &str) -> Value {
    json!({ "code": "validation", "fields": { field: "not_set" } })
}

/// A Google-only account (registration on) and its base-host session.
async fn google_only(app: &axum::Router, mock: &common::oidc::OidcMock) -> String {
    let done = sign_in(
        app,
        mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-gonly",
        EMAIL,
    )
    .await;
    done.session.expect("session")
}

async fn call(
    app: &axum::Router,
    m: Method,
    host: &str,
    uri: &str,
    cookie: &str,
    body: Value,
) -> (StatusCode, Value) {
    on(app, m, host, uri, As::Cookie(cookie), Some(body)).await
}

#[tokio::test]
async fn google_only_account_has_no_password() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, true);
    let base = google_only(&app, &mock).await;
    for pw in ["", "anything at all here"] {
        let err = login(&app, BASE_HOST, EMAIL, pw)
            .await
            .expect_err("refused");
        assert_eq!(
            err,
            (
                StatusCode::UNAUTHORIZED,
                json!({ "code": "invalid_credentials" })
            )
        );
    }
    // Every password-requiring action: `password: not_set`.
    let pw = json!({ "password": "whatever it is" });
    for uri in ["/api/account/mfa/setup", "/api/account/mfa/disable"] {
        let (s, j) = call(&app, Method::POST, BASE_HOST, uri, &base, pw.clone()).await;
        assert_eq!(
            (s, j),
            (StatusCode::UNPROCESSABLE_ENTITY, not_set("password")),
            "{uri}"
        );
    }
    let (s, j) = call(
        &app,
        Method::DELETE,
        BASE_HOST,
        "/api/account/google",
        &base,
        pw.clone(),
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::UNPROCESSABLE_ENTITY, not_set("password"))
    );
    // Space delete (an own space, signed in on its host through Google).
    let (s, _) = call(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/spaces",
        &base,
        json!({ "slug": "gspace", "name": "G" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let host = "gspace.localhost:3000";
    let done = sign_in(&app, &mock, host, "purpose=login", None, "sub-gonly", EMAIL).await;
    let space = done.session.expect("space session");
    let body = json!({ "slug": "gspace", "password": "x" });
    let (s, j) = call(&app, Method::DELETE, host, "/api/space", &space, body).await;
    assert_eq!(
        (s, j),
        (StatusCode::UNPROCESSABLE_ENTITY, not_set("password"))
    );
    // Invitation accept with the existing (password-less) account.
    let token = invited(&app, EMAIL, "member").await;
    let (s, _, j, _) =
        common::members::accept(&app, TEST_HOST, json!({ "token": token, "password": "x" })).await;
    assert_eq!(
        (s, j),
        (StatusCode::UNPROCESSABLE_ENTITY, not_set("password"))
    );
}

#[tokio::test]
async fn set_password_within_ten_minutes_of_a_google_login() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, true);
    let base = google_only(&app, &mock).await;
    let other = google_only(&app, &mock).await;
    let short = json!({ "newPassword": "short" });
    let (s, j) = call(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/account/password",
        &base,
        short,
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "newPassword": "too_short" })
        )
    );
    let body = json!({ "newPassword": NEW_PASSWORD });
    let (s, j) = call(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/account/password",
        &base,
        body,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    assert!(login(&app, BASE_HOST, EMAIL, NEW_PASSWORD).await.is_ok());
    let (_, me) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/auth/me",
        As::Cookie(&base),
        None,
    )
    .await;
    assert_eq!(me["user"]["hasPassword"], true);
    // As any password change: the other sessions are gone.
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
    // From now on the current password is needed.
    let body = json!({ "newPassword": "yet another password" });
    let (s, j) = call(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/account/password",
        &base,
        body,
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "currentPassword": "invalid" })
        )
    );
}

#[tokio::test]
async fn set_password_after_ten_minutes_needs_the_reset() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, true);
    let base = google_only(&app, &mock).await;
    db.conn
        .execute_unprepared("UPDATE sessions SET created_at = now() - interval '11 minutes'")
        .await
        .expect("age the session");
    let body = json!({ "newPassword": NEW_PASSWORD });
    let (s, j) = call(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/account/password",
        &base,
        body.clone(),
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "currentPassword": "required" })
        )
    );
    // A session not created by Google does not count either.
    db.conn
        .execute_unprepared("UPDATE sessions SET created_at = now(), login_method = 'password'")
        .await
        .expect("password session");
    let (s, j) = call(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/account/password",
        &base,
        body,
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "currentPassword": "required" })
        )
    );
}

#[tokio::test]
async fn unlink_rules() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let enrolled = common::mfa::enrol(&app, OWNER_EMAIL).await;
    let cookie = enrolled.cookie.clone();
    // Starting a link re-confirms the password and, with TOTP, a code.
    let (s, j) = google::link_request(&app, BASE_HOST, &cookie, None).await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "code": "required" })
        )
    );
    let code = common::mfa::fresh(&db.conn, db.user_id, &enrolled.secret).await;
    let (s, j) = google::link_request(&app, BASE_HOST, &cookie, Some(&code)).await;
    assert_eq!(s, StatusCode::OK, "{j}");
    let url = j["url"].as_str().expect("url");
    let q = url
        .split_once('?')
        .map(|(_, q)| q.to_string())
        .expect("query");
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        &q,
        Some(&cookie),
        "sub-owner",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(
        done.location,
        common::google::at(BASE_HOST, "/account?google=linked")
    );
    let uri = "/api/account/google";
    let (s, j) = call(
        &app,
        Method::DELETE,
        BASE_HOST,
        uri,
        &cookie,
        json!({ "password": "wrong password!" }),
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "password": "invalid", "code": "required" })
        )
    );
    let (s, j) = call(
        &app,
        Method::DELETE,
        BASE_HOST,
        uri,
        &cookie,
        json!({ "password": OWNER_PASSWORD }),
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "code": "required" })
        )
    );
    let code = common::mfa::fresh(&db.conn, db.user_id, &enrolled.secret).await;
    let body = json!({ "password": OWNER_PASSWORD, "code": code });
    let (s, j) = call(&app, Method::DELETE, BASE_HOST, uri, &cookie, body).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    let (_, me) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/auth/me",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(me["user"]["googleLinked"], false);
    // A token cannot unlink (session only).
    let (s, _) = on(
        &app,
        Method::DELETE,
        TEST_HOST,
        uri,
        As::Bearer(common::TEST_TOKEN),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
