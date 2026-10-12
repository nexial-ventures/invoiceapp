//! Step-up codes (mfa.md, 4c) on password change, space delete and token
//! create; users without TOTP are unchanged. (Invite accept, disable and
//! recovery codes: `mfa_policy.rs`, `mfa_enrol.rs`.)

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, member, on};
use common::members::session_on;
use common::mfa::{enrol, finish, fresh, require_mfa, start};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb, router};
use invoice::auth::users;
use invoice::space::Role;
use serde_json::{Value, json};

fn fields(v: Value) -> (StatusCode, Value) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "code": "validation", "fields": v }),
    )
}

#[tokio::test]
async fn password_change_needs_a_code() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let change = |code: Option<&str>, new: &str| json!({ "currentPassword": OWNER_PASSWORD, "newPassword": new, "code": code });
    let call = async |body: Value| {
        on(
            &app,
            Method::POST,
            BASE_HOST,
            "/api/account/password",
            As::Cookie(&e.cookie),
            Some(body),
        )
        .await
    };
    // `required` is reported with the other errors; nothing is spent then.
    assert_eq!(
        call(change(None, "short")).await,
        fields(json!({ "code": "required", "newPassword": "too_short" }))
    );
    let new = "a brand new long password";
    assert_eq!(
        call(change(Some("000000x"), new)).await,
        fields(json!({ "code": "invalid" }))
    );
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    assert_eq!(
        call(change(Some(&code), new)).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn space_delete_needs_a_code() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (_, cookie, _, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    let cookie = cookie.expect("session");
    let del =
        |code: Option<&str>| json!({ "slug": "acme", "password": OWNER_PASSWORD, "code": code });
    let call = async |body: Value| {
        on(
            &app,
            Method::DELETE,
            TEST_HOST,
            "/api/space",
            As::Cookie(&cookie),
            Some(body),
        )
        .await
    };
    assert_eq!(call(del(None)).await, fields(json!({ "code": "required" })));
    assert_eq!(
        call(del(Some("123456"))).await,
        fields(json!({ "code": "invalid" }))
    );
    // A wrong slug: reported, the (valid) code not spent.
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let wrong = json!({ "slug": "nope", "password": OWNER_PASSWORD, "code": code });
    assert_eq!(call(wrong).await, fields(json!({ "slug": "mismatch" })));
    assert_eq!(call(del(Some(&code))).await.0, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn token_create_needs_a_code() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let call = async |body: Value| {
        on(
            &app,
            Method::POST,
            TEST_HOST,
            "/api/tokens",
            As::Bearer(TEST_TOKEN),
            Some(body),
        )
        .await
    };
    assert_eq!(
        call(json!({ "name": "", "role": "member" })).await,
        fields(json!({ "code": "required", "name": "required" }))
    );
    assert_eq!(
        call(json!({ "name": "MCP", "role": "member", "code": "654321" })).await,
        fields(json!({ "code": "invalid" }))
    );
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (s, j) = call(json!({ "name": "MCP", "role": "member", "code": code })).await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    // A recovery code works too.
    let (s, _) = call(json!({ "name": "MCP 2", "role": "member", "code": e.codes[0] })).await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn users_without_totp_are_unchanged() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let token = member(&db.conn, db.space, "clen@example.com", Role::Member).await;
    // A sent code is ignored.
    let body = json!({ "name": "x", "role": "member", "code": "garbage" });
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(&token),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(&token),
        Some(json!({ "name": "y", "role": "member" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

/// Password change, reset and "sign out elsewhere" also end the user's
/// half-finished (pending) logins.
#[tokio::test]
async fn password_events_drop_pending_logins() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let invalid = (
        StatusCode::UNAUTHORIZED,
        json!({ "code": "invalid_credentials" }),
    );
    let dead = async |pending: &str| {
        let code = fresh(&db.conn, db.user_id, &e.secret).await;
        let (s, _, j, _) = finish(&app, TEST_HOST, Some(pending), &code).await;
        (s, j)
    };

    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/sessions/revoke-others",
        As::Cookie(&e.cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(dead(&pending).await, invalid);

    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let body =
        json!({ "currentPassword": OWNER_PASSWORD, "newPassword": OWNER_PASSWORD, "code": code });
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/account/password",
        As::Cookie(&e.cookie),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(dead(&pending).await, invalid);

    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let reset = users::issue_token(&db.conn, db.user_id, users::TokenKind::Reset)
        .await
        .expect("reset token");
    let body = json!({ "token": reset, "password": OWNER_PASSWORD });
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/password-reset/confirm",
        As::Nobody,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(dead(&pending).await, invalid);
}

/// Disabling while TOTP is off: the password is checked, nothing is wiped.
#[tokio::test]
async fn disable_without_totp_touches_nothing() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    require_mfa(&db.conn, "acme").await;
    let session = session_on(&db.conn, db.space, db.user_id).await;
    let base = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let path = "/api/account/mfa/disable";
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        path,
        As::Cookie(&base),
        Some(json!({ "password": "wrong" })),
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "password": "invalid" })
        )
    );
    let body = json!({ "password": OWNER_PASSWORD, "code": "ignored" });
    let (s, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        path,
        As::Cookie(&base),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // New recovery codes need TOTP on: a typed code, not a generic conflict.
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/account/mfa/recovery-codes",
        As::Cookie(&base),
        Some(json!({ "password": OWNER_PASSWORD, "code": "123456" })),
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::CONFLICT, json!({ "code": "mfa_disabled" }))
    );
    for who in [As::Cookie(&session), As::Bearer(TEST_TOKEN)] {
        let (s, _) = on(&app, Method::GET, TEST_HOST, "/api/auth/me", who, None).await;
        assert_eq!(s, StatusCode::OK);
    }
}
