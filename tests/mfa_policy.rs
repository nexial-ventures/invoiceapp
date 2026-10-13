//! The per-space TOTP policy (mfa.md, 4c): owner only, the owner needs
//! TOTP, refused next logins, existing credentials keep working, members'
//! `mfaEnabled`, and the invitation accept outcomes.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, member, on, user};
use common::members::{accept, count, invited, show};
use common::mfa::{enrol, fresh, require_mfa};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb, router};
use invoiceapp::space::Role;
use serde_json::{Value, json};

fn mfa_required() -> (StatusCode, Value) {
    (StatusCode::FORBIDDEN, json!({ "code": "mfa_required" }))
}

async fn put_space(app: &axum::Router, who: As<'_>, body: Value) -> (StatusCode, Value) {
    on(app, Method::PUT, TEST_HOST, "/api/space", who, Some(body)).await
}

#[tokio::test]
async fn only_an_owner_session_with_totp_and_a_code_changes_the_policy() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let forbidden = (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }));
    let on_ = json!({ "requireMfa": true });
    assert_eq!(
        put_space(&app, As::Bearer(&admin), on_.clone()).await,
        forbidden
    );
    assert_eq!(
        put_space(&app, As::Bearer(&admin), json!({ "requireMfa": false })).await,
        forbidden
    );
    // An admin may still rename (with a token, without a code).
    let (s, j) = put_space(&app, As::Bearer(&admin), json!({ "name": "Acme" })).await;
    assert_eq!((s, j["requireMfa"].clone()), (StatusCode::OK, json!(false)));
    // The owner's token: session only.
    assert_eq!(
        put_space(&app, As::Bearer(TEST_TOKEN), on_.clone()).await,
        forbidden
    );
    // The owner's session without TOTP: on and off both need it.
    let plain = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let not_enabled = (
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "code": "validation", "fields": { "requireMfa": "mfa_not_enabled" } }),
    );
    assert_eq!(
        put_space(&app, As::Cookie(&plain), on_.clone()).await,
        not_enabled
    );
    let off = json!({ "requireMfa": false, "code": "123456" });
    assert_eq!(put_space(&app, As::Cookie(&plain), off).await, not_enabled);

    let e = enrol(&app, OWNER_EMAIL).await;
    let pending = common::mfa::start(&app, TEST_HOST, OWNER_EMAIL).await;
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (_, cookie, _, _) = common::mfa::finish(&app, TEST_HOST, Some(&pending), &code).await;
    let cookie = cookie.expect("session");
    let field = |r: &str| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "code": "validation", "fields": { "code": r } }),
        )
    };
    assert_eq!(
        put_space(&app, As::Cookie(&cookie), on_).await,
        field("required")
    );
    let wrong = json!({ "requireMfa": true, "code": "000000" });
    assert_eq!(
        put_space(&app, As::Cookie(&cookie), wrong).await,
        field("invalid")
    );
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let body = json!({ "requireMfa": true, "code": code });
    let (s, j) = put_space(&app, As::Cookie(&cookie), body).await;
    assert_eq!(
        (s, j["requireMfa"].clone(), j["name"].clone()),
        (StatusCode::OK, json!(true), json!("Acme"))
    );
    let (_, j) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/space",
        As::Bearer(TEST_TOKEN),
        None,
    )
    .await;
    assert_eq!(j["requireMfa"], json!(true));
    // An empty body changes nothing; turning it off needs a code too.
    let (s, j) = put_space(&app, As::Bearer(TEST_TOKEN), json!({})).await;
    assert_eq!((s, j["requireMfa"].clone()), (StatusCode::OK, json!(true)));
    let off = json!({ "requireMfa": false, "code": e.codes[0] });
    let (s, j) = put_space(&app, As::Cookie(&cookie), off).await;
    assert_eq!((s, j["requireMfa"].clone()), (StatusCode::OK, json!(false)));
}

#[tokio::test]
async fn new_tokens_follow_the_policy() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let email = "clen@example.com";
    let token = member(&db.conn, db.space, email, Role::Member).await;
    let session = login(&app, TEST_HOST, email, OWNER_PASSWORD)
        .await
        .expect("login");
    require_mfa(&db.conn, "acme").await;
    let body = json!({ "name": "MCP", "role": "member" });
    for who in [As::Bearer(&token), As::Cookie(&session)] {
        let r = on(
            &app,
            Method::POST,
            TEST_HOST,
            "/api/tokens",
            who,
            Some(body.clone()),
        )
        .await;
        assert_eq!(r, mfa_required());
    }
    // With TOTP (and its code) it works.
    let e = enrol(&app, OWNER_EMAIL).await;
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let body = json!({ "name": "MCP", "role": "member", "code": code });
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(TEST_TOKEN),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn the_policy_refuses_the_next_login_only() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let email = "clen@example.com";
    let token = member(&db.conn, db.space, email, Role::Member).await;
    let before = login(&app, TEST_HOST, email, OWNER_PASSWORD)
        .await
        .expect("login");
    require_mfa(&db.conn, "acme").await;

    assert_eq!(
        login(&app, TEST_HOST, email, OWNER_PASSWORD).await,
        Err(mfa_required())
    );
    // Only after a correct password.
    let err = login(&app, TEST_HOST, email, "wrong password!")
        .await
        .expect_err("401");
    assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    assert_eq!(count(&db.conn, "SELECT count(*) FROM sessions").await, 1);
    // Existing credentials keep working; the base host is not affected.
    let me = |who| on(&app, Method::GET, TEST_HOST, "/api/auth/me", who, None);
    assert_eq!(me(As::Cookie(&before)).await.0, StatusCode::OK);
    assert_eq!(me(As::Bearer(&token)).await.0, StatusCode::OK);
    assert!(login(&app, BASE_HOST, email, OWNER_PASSWORD).await.is_ok());

    // With TOTP the login goes through the code step.
    let e = enrol(&app, email).await;
    let pending = common::mfa::start(&app, TEST_HOST, email).await;
    let uid = common::members::user_id(&db.conn, email).await;
    let code = fresh(&db.conn, uid, &e.secret).await;
    let (s, cookie, _, _) = common::mfa::finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!((s, cookie.is_some()), (StatusCode::NO_CONTENT, true));
}

#[tokio::test]
async fn members_show_mfa_enabled() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    member(&db.conn, db.space, "bez@example.com", Role::Member).await;
    enrol(&app, OWNER_EMAIL).await;
    let (s, j) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/members",
        As::Bearer(TEST_TOKEN),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let flags: Vec<(String, bool)> = j
        .as_array()
        .expect("list")
        .iter()
        .map(|m| {
            (
                m["email"].as_str().unwrap_or_default().into(),
                m["mfaEnabled"] == json!(true),
            )
        })
        .collect();
    assert_eq!(
        flags,
        vec![
            (OWNER_EMAIL.into(), true),
            ("bez@example.com".into(), false)
        ]
    );
}

#[tokio::test]
async fn invite_into_a_requiring_space() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    enrol(&app, OWNER_EMAIL).await;
    require_mfa(&db.conn, "acme").await;

    // A new account: created (verified), not joined, the link stays valid.
    let token = invited(&app, "nova@example.com", "member").await;
    let (s, info) = show(&app, &token).await;
    assert_eq!(
        (s, info["requireMfa"].clone()),
        (StatusCode::OK, json!(true))
    );
    let body = json!({ "token": token, "password": OWNER_PASSWORD, "displayName": "Nová" });
    let (s, cookie, j, _) = accept(&app, TEST_HOST, body.clone()).await;
    assert_eq!(
        (s, j),
        (
            StatusCode::FORBIDDEN,
            json!({ "code": "mfa_required", "detail": "account_created" })
        )
    );
    assert!(cookie.is_none());
    let joined = "SELECT count(*) FROM space_members m JOIN users u ON u.id = m.user_id \
                  WHERE u.email = 'nova@example.com'";
    assert_eq!(count(&db.conn, joined).await, 0);
    assert_eq!(show(&app, &token).await.1["accountExists"], json!(true));

    // Now an existing account without TOTP: refused (a `code` is ignored).
    let body = json!({ "token": token, "password": OWNER_PASSWORD, "code": "123456" });
    let (s, _, j, _) = accept(&app, TEST_HOST, body.clone()).await;
    assert_eq!((s, j), mfa_required());
    // A wrong password is still 401 first.
    let wrong = json!({ "token": token, "password": "wrong password!" });
    assert_eq!(
        accept(&app, TEST_HOST, wrong).await.0,
        StatusCode::UNAUTHORIZED
    );

    // It signs in on the base host, enables TOTP and opens the link again.
    let e = enrol(&app, "nova@example.com").await;
    let uid = common::members::user_id(&db.conn, "nova@example.com").await;
    let field = |r: &str| json!({ "code": "validation", "fields": { "code": r } });
    let (s, _, j, _) = accept(
        &app,
        TEST_HOST,
        json!({ "token": token, "password": OWNER_PASSWORD }),
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::UNPROCESSABLE_ENTITY, field("required"))
    );
    let (s, _, j, _) = accept(&app, TEST_HOST, body).await;
    assert_eq!((s, j), (StatusCode::UNPROCESSABLE_ENTITY, field("invalid")));
    let code = fresh(&db.conn, uid, &e.secret).await;
    let ok = json!({ "token": token, "password": OWNER_PASSWORD, "code": code });
    let (s, cookie, _, _) = accept(&app, TEST_HOST, ok).await;
    assert_eq!((s, cookie.is_some()), (StatusCode::NO_CONTENT, true));
    assert_eq!(count(&db.conn, joined).await, 1);
}

#[tokio::test]
async fn invite_without_the_policy_needs_the_code_of_a_totp_account() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let uid = user(&db.conn, "jana@example.com").await;
    let e = enrol(&app, "jana@example.com").await;
    let token = invited(&app, "jana@example.com", "accountant").await;
    assert_eq!(show(&app, &token).await.1["requireMfa"], json!(false));
    let (s, _, j, _) = accept(
        &app,
        TEST_HOST,
        json!({ "token": token, "password": OWNER_PASSWORD }),
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "code": "required" })
        )
    );
    let code = fresh(&db.conn, uid, &e.secret).await;
    let (s, _, _, _) = accept(
        &app,
        TEST_HOST,
        json!({ "token": token, "password": OWNER_PASSWORD, "code": code }),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // A user without TOTP is unchanged (a sent code is ignored).
    user(&db.conn, "petr@example.com").await;
    let token = invited(&app, "petr@example.com", "member").await;
    let body = json!({ "token": token, "password": OWNER_PASSWORD, "code": "nonsense" });
    assert_eq!(
        accept(&app, TEST_HOST, body).await.0,
        StatusCode::NO_CONTENT
    );
}
