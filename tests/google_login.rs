//! Sign in with Google (google.md, 4d): the login purpose on the base host
//! and on space hosts, account resolution, provider failures, disabled
//! configuration.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, on};
use common::google::{self, at, sign_in};
use common::members::count;
use common::oidc::claims;
use common::{BASE_HOST, OWNER_EMAIL, TEST_HOST, TestDb};
use sea_orm::ConnectionTrait;
use serde_json::json;

async fn me(app: &axum::Router, host: &str, cookie: &str) -> serde_json::Value {
    let (s, j) = on(
        app,
        Method::GET,
        host,
        "/api/auth/me",
        As::Cookie(cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{j}");
    j
}

#[tokio::test]
async fn disabled_configuration_hides_everything() {
    let db = TestDb::new().await;
    let app = common::router(db.conn.clone());
    for (host, uri) in [
        (BASE_HOST, "/api/auth/google/start?purpose=login"),
        (TEST_HOST, "/api/auth/google/start?purpose=login"),
        (BASE_HOST, "/api/auth/google/callback?state=x&code=y"),
        (TEST_HOST, "/api/auth/google/finish?token=x"),
    ] {
        let (s, j) = on(&app, Method::GET, host, uri, As::Nobody, None).await;
        assert_eq!(
            (s, j),
            (StatusCode::NOT_FOUND, json!({ "code": "not_found" })),
            "{uri}"
        );
    }
    let (_, ctx) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/context",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(ctx["google"], false);
    let cookie = common::auth::login(&app, BASE_HOST, OWNER_EMAIL, common::OWNER_PASSWORD)
        .await
        .expect("login");
    let body = json!({ "password": common::OWNER_PASSWORD });
    let (s, _) = on(
        &app,
        Method::DELETE,
        BASE_HOST,
        "/api/account/google",
        As::Cookie(&cookie),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn base_login_links_the_account_by_email_then_by_sub() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let (_, ctx) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/context",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(ctx["google"], true);
    // The callback is on the base host only.
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/google/callback?state=x",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login&return=/spaces",
        None,
        "sub-owner",
        "Owner@Example.com",
    )
    .await;
    assert_eq!(done.location, at(BASE_HOST, "/spaces"));
    let cookie = done.session.expect("session");
    let me = me(&app, BASE_HOST, &cookie).await;
    assert_eq!(me["user"]["email"], OWNER_EMAIL);
    assert_eq!(
        (
            me["user"]["googleLinked"].clone(),
            me["user"]["hasPassword"].clone()
        ),
        (json!(true), json!(true))
    );
    let method = count(
        &db.conn,
        "SELECT count(*) FROM sessions WHERE login_method = 'google'",
    )
    .await;
    assert_eq!(method, 1);
    // The password still works.
    assert!(
        common::auth::login(&app, BASE_HOST, OWNER_EMAIL, common::OWNER_PASSWORD)
            .await
            .is_ok()
    );
    // Next time by `sub`, whatever e-mail Google reports.
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login&return=//evil.example.com",
        None,
        "sub-owner",
        "renamed@example.com",
    )
    .await;
    assert_eq!(done.location, at(BASE_HOST, "/"), "return sanitised");
    assert!(done.session.is_some());
    // Another Google account with the owner's e-mail: already linked.
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-other",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(
        done.location,
        at(BASE_HOST, "/login?google_error=already_linked")
    );
}

#[tokio::test]
async fn space_login_needs_membership() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, true);
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login&return=/documents",
        None,
        "sub-owner",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(done.location, at(TEST_HOST, "/documents"));
    let cookie = done.session.expect("session");
    assert_eq!(me(&app, TEST_HOST, &cookie).await["space"]["role"], "owner");
    // The session belongs to that host only.
    let (s, _) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/auth/me",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    common::auth::user(&db.conn, "outsider@example.com").await;
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "sub-out",
        "outsider@example.com",
    )
    .await;
    assert_eq!(
        done.location,
        at(TEST_HOST, "/login?google_error=not_member")
    );
    assert!(done.session.is_none());
    // No account behind a space host's login: none is created, even with
    // registration on.
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "sub-new",
        "nobody@example.com",
    )
    .await;
    assert_eq!(
        done.location,
        at(TEST_HOST, "/login?google_error=not_member")
    );
    assert_eq!(
        count(
            &db.conn,
            "SELECT count(*) FROM users WHERE email = 'nobody@example.com'"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn new_accounts_only_with_registration() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-new",
        "new@example.com",
    )
    .await;
    assert_eq!(
        done.location,
        at(BASE_HOST, "/login?google_error=not_allowed")
    );
    assert_eq!(
        count(
            &db.conn,
            "SELECT count(*) FROM users WHERE email = 'new@example.com'"
        )
        .await,
        0
    );

    let (app, mock) = google::app(&db.conn, true);
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-new",
        "New@Example.com",
    )
    .await;
    let cookie = done.session.expect("session");
    let user = me(&app, BASE_HOST, &cookie).await;
    assert_eq!(user["user"]["email"], "new@example.com");
    assert_eq!(user["user"]["displayName"], "Jana Googlová");
    assert_eq!(user["user"]["emailVerified"], true);
    assert_eq!(user["user"]["hasPassword"], false);
    assert_eq!(user["user"]["googleLinked"], true);
    // A verified account may create a space.
    let body = json!({ "slug": "nova", "name": "Nová" });
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&cookie),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    // Without a name: the e-mail's local part.
    let st = google::start(&app, &mock, BASE_HOST, "purpose=login", &[]).await;
    let mut c = claims(&mock, "sub-noname", "petr@example.com", &st.nonce);
    c.as_object_mut().expect("claims").remove("name");
    let to = google::callback(&app, &mock, &st, c).await;
    let done = google::finish(&app, &to, BASE_HOST, &[st.browser]).await;
    let user = me(&app, BASE_HOST, &done.session.expect("session")).await;
    assert_eq!(user["user"]["displayName"], "petr");
}

#[tokio::test]
async fn disabled_and_unverified_users() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, true);
    common::auth::user(&db.conn, "off@example.com").await;
    db.conn
        .execute_unprepared("UPDATE users SET disabled = true WHERE email = 'off@example.com'")
        .await
        .expect("disable");
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-off",
        "off@example.com",
    )
    .await;
    assert_eq!(done.location, at(BASE_HOST, "/login?google_error=disabled"));

    let st = google::start(&app, &mock, BASE_HOST, "purpose=login", &[]).await;
    let mut c = claims(&mock, "sub-x", "x@example.com", &st.nonce);
    c["email_verified"] = json!(false);
    let to = google::callback(&app, &mock, &st, c).await;
    assert_eq!(to, at(BASE_HOST, "/login?google_error=unverified_email"));
}

/// Someone registered the address without proving it: the Google owner
/// gets the account, the registrant's password goes.
#[tokio::test]
async fn unverified_registration_is_claimed() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let id = common::auth::user(&db.conn, "claim@example.com").await;
    db.conn
        .execute_unprepared(
            "UPDATE users SET email_verified_at = NULL WHERE email = 'claim@example.com'",
        )
        .await
        .expect("unverify");
    let old = common::auth::login(&app, BASE_HOST, "claim@example.com", common::OWNER_PASSWORD)
        .await
        .expect("an unverified user logs in on the base host");
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-claim",
        "claim@example.com",
    )
    .await;
    let me = me(&app, BASE_HOST, &done.session.expect("session")).await;
    assert_eq!(me["user"]["id"], id.to_string());
    assert_eq!(
        (
            me["user"]["emailVerified"].clone(),
            me["user"]["hasPassword"].clone()
        ),
        (json!(true), json!(false))
    );
    let (s, _) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/auth/me",
        As::Cookie(&old),
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "the registrant's session ended"
    );
    let err =
        common::auth::login(&app, BASE_HOST, "claim@example.com", common::OWNER_PASSWORD).await;
    assert_eq!(err.map_err(|e| e.0), Err(StatusCode::UNAUTHORIZED));
}
