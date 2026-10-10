//! Sign in with Google (google.md, 4d): TOTP and the space policy,
//! invitations, linking from the account page.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, on};
use common::google::{self, at, sign_in};
use common::members::{count, invited};
use common::mfa::{enrol, finish, fresh, require_mfa};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_SLUG, TestDb};
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
async fn totp_user_continues_with_the_code_step() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let enrolled = enrol(&app, OWNER_EMAIL).await;
    for host in [TEST_HOST, BASE_HOST] {
        let done = sign_in(
            &app,
            &mock,
            host,
            "purpose=login",
            None,
            "sub-owner",
            OWNER_EMAIL,
        )
        .await;
        assert_eq!(done.location, at(host, "/login?mfa=1"));
        assert!(done.session.is_none());
        let pending = done.pending.expect("invoice_mfa");
        let code = fresh(&db.conn, db.user_id, &enrolled.secret).await;
        let (s, session, j, _) = finish(&app, host, Some(&pending), &code).await;
        assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
        me(&app, host, &session.expect("session")).await;
    }
    let google = count(
        &db.conn,
        "SELECT count(*) FROM sessions WHERE login_method = 'google'",
    )
    .await;
    assert_eq!(google, 2, "the session records the Google login");
}

#[tokio::test]
async fn requiring_space_refuses_users_without_totp() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    require_mfa(&db.conn, TEST_SLUG).await;
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "sub-owner",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(
        done.location,
        at(TEST_HOST, "/login?google_error=mfa_required")
    );
    assert!(done.session.is_none() && done.pending.is_none());
    // The base host has no policy.
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-owner",
        OWNER_EMAIL,
    )
    .await;
    assert!(done.session.is_some());
}

#[tokio::test]
async fn invite_creates_the_account_and_joins() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let token = invited(&app, "new@example.com", "member").await;
    let q = format!("purpose=invite&invite={token}&return=/documents");
    // Another Google address: refused before any account exists.
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        &q,
        None,
        "sub-x",
        "other@example.com",
    )
    .await;
    assert_eq!(
        done.location,
        at(
            TEST_HOST,
            &format!("/invite?token={token}&google_error=email_mismatch")
        )
    );
    assert_eq!(
        count(
            &db.conn,
            "SELECT count(*) FROM users WHERE email = 'other@example.com'"
        )
        .await,
        0
    );

    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        &q,
        None,
        "sub-new",
        "New@Example.com",
    )
    .await;
    assert_eq!(done.location, at(TEST_HOST, "/documents"));
    let user = me(&app, TEST_HOST, &done.session.expect("session")).await;
    assert_eq!(user["space"]["role"], "member");
    assert_eq!(
        (
            user["user"]["hasPassword"].clone(),
            user["user"]["emailVerified"].clone()
        ),
        (json!(false), json!(true))
    );
    assert_eq!(
        count(&db.conn, "SELECT count(*) FROM space_invites").await,
        0
    );
    // Used up.
    let (s, h, _) = google::get(&app, TEST_HOST, &format!("/api/auth/google/start?{q}"), &[]).await;
    let expected = at(
        TEST_HOST,
        &format!("/invite?token={token}&google_error=invalid"),
    );
    assert_eq!((s, google::location(&h)), (StatusCode::FOUND, expected));
}

#[tokio::test]
async fn invite_with_an_existing_account() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    common::auth::user(&db.conn, "jana@example.com").await;
    let token = invited(&app, "jana@example.com", "accountant").await;
    let q = format!("purpose=invite&invite={token}");
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        &q,
        None,
        "sub-jana",
        "jana@example.com",
    )
    .await;
    assert_eq!(done.location, at(TEST_HOST, "/"));
    let user = me(&app, TEST_HOST, &done.session.expect("session")).await;
    assert_eq!(user["space"]["role"], "accountant");
    assert_eq!(user["user"]["googleLinked"], true);
}

#[tokio::test]
async fn invite_into_a_requiring_space() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    require_mfa(&db.conn, TEST_SLUG).await;
    // Without TOTP: refused, the invitation stays (the account is created).
    let token = invited(&app, "plain@example.com", "member").await;
    let q = format!("purpose=invite&invite={token}");
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        &q,
        None,
        "sub-plain",
        "plain@example.com",
    )
    .await;
    assert_eq!(
        done.location,
        at(
            TEST_HOST,
            &format!("/invite?token={token}&google_error=mfa_required")
        )
    );
    assert_eq!(
        count(
            &db.conn,
            "SELECT count(*) FROM space_invites WHERE email = 'plain@example.com'"
        )
        .await,
        1
    );

    // With TOTP: the code step; the membership only after the right code.
    let id = common::auth::user(&db.conn, "totp@example.com").await;
    let enrolled = enrol(&app, "totp@example.com").await;
    let token = invited(&app, "totp@example.com", "member").await;
    let q = format!("purpose=invite&invite={token}");
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        &q,
        None,
        "sub-totp",
        "totp@example.com",
    )
    .await;
    assert_eq!(done.location, at(TEST_HOST, "/login?mfa=1"));
    let pending = done.pending.expect("invoice_mfa");
    let members = "SELECT count(*) FROM space_members m JOIN users u ON u.id = m.user_id \
                   WHERE u.email = 'totp@example.com'";
    let invites = "SELECT count(*) FROM space_invites WHERE email = 'totp@example.com'";
    let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), "000000").await;
    assert_eq!(
        (s, j),
        (StatusCode::UNAUTHORIZED, json!({ "code": "mfa_invalid" }))
    );
    assert_eq!(
        (
            count(&db.conn, members).await,
            count(&db.conn, invites).await
        ),
        (0, 1)
    );
    let code = fresh(&db.conn, id, &enrolled.secret).await;
    let (s, session, j, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    assert_eq!(
        me(&app, TEST_HOST, &session.expect("session")).await["space"]["role"],
        "member"
    );
    assert_eq!(
        (
            count(&db.conn, members).await,
            count(&db.conn, invites).await
        ),
        (1, 0)
    );
}

#[tokio::test]
async fn link_from_the_account_page() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let base = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    // Any verified Google address may be linked.
    let q = google::link_query(&app, BASE_HOST, &base).await;
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        &q,
        Some(&base),
        "sub-owner",
        "private@gmail.example",
    )
    .await;
    assert_eq!(done.location, at(BASE_HOST, "/account?google=linked"));
    assert!(done.session.is_none(), "no new login");
    assert_eq!(
        me(&app, BASE_HOST, &base).await["user"]["googleLinked"],
        true
    );
    // Google login with it signs in as the owner.
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        "purpose=login",
        None,
        "sub-owner",
        "private@gmail.example",
    )
    .await;
    assert_eq!(
        me(&app, BASE_HOST, &done.session.expect("session")).await["user"]["email"],
        OWNER_EMAIL
    );

    // That Google account belongs to the owner now.
    common::auth::user(&db.conn, "second@example.com").await;
    let other = login(&app, BASE_HOST, "second@example.com", OWNER_PASSWORD)
        .await
        .expect("login");
    let q = google::link_query(&app, BASE_HOST, &other).await;
    let done = sign_in(
        &app,
        &mock,
        BASE_HOST,
        &q,
        Some(&other),
        "sub-owner",
        "x@example.com",
    )
    .await;
    assert_eq!(
        done.location,
        at(BASE_HOST, "/account?google_error=already_linked")
    );
    assert_eq!(
        me(&app, BASE_HOST, &other).await["user"]["googleLinked"],
        false
    );
}
