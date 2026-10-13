//! TOTP enrolment, status, recovery codes and disabling (mfa.md, 4c).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, on};
use common::members::{B_HOST, count, join, session_on, token};
use common::mfa::{base32_decode, code_now, enrol, fresh, require_mfa};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb, router};
use invoiceapp::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

async fn post(app: &axum::Router, cookie: &str, path: &str, body: Value) -> (StatusCode, Value) {
    on(
        app,
        Method::POST,
        BASE_HOST,
        path,
        As::Cookie(cookie),
        Some(body),
    )
    .await
}

async fn me_status(app: &axum::Router, host: &str, who: As<'_>) -> StatusCode {
    on(app, Method::GET, host, "/api/auth/me", who, None)
        .await
        .0
}

fn field(name: &str, reason: &str) -> (StatusCode, Value) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "code": "validation", "fields": { name: reason } }),
    )
}

#[tokio::test]
async fn setup_then_enable() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let cookie = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let me = async |c: &str| {
        on(
            &app,
            Method::GET,
            BASE_HOST,
            "/api/auth/me",
            As::Cookie(c),
            None,
        )
        .await
        .1
    };
    assert_eq!(me(&cookie).await["user"]["mfaEnabled"], json!(false));

    let setup = json!({ "password": OWNER_PASSWORD });
    assert_eq!(
        post(
            &app,
            &cookie,
            "/api/account/mfa/setup",
            json!({ "password": "wrong" })
        )
        .await,
        field("password", "invalid")
    );
    let (s, j) = post(&app, &cookie, "/api/account/mfa/setup", setup.clone()).await;
    assert_eq!(s, StatusCode::OK);
    let b32 = j["secret"].as_str().expect("secret");
    assert_eq!(b32.len(), 32, "20 bytes");
    assert_eq!(
        j["otpauthUri"],
        json!(format!(
            "otpauth://totp/InvoiceApp%20(localhost):{OWNER_EMAIL}?secret={b32}\
             &issuer=InvoiceApp%20(localhost)&digits=6&period=30"
        ))
    );
    // A new setup replaces the pending secret: the first one's codes fail.
    let first = base32_decode(b32);
    let (_, j) = post(&app, &cookie, "/api/account/mfa/setup", setup.clone()).await;
    let secret = base32_decode(j["secret"].as_str().expect("secret"));
    assert_ne!(first, secret);
    let enable = |code: String| json!({ "code": code });
    assert_eq!(
        post(
            &app,
            &cookie,
            "/api/account/mfa/enable",
            enable(code_now(&first))
        )
        .await,
        field("code", "invalid")
    );
    assert_eq!(
        post(&app, &cookie, "/api/account/mfa/enable", json!({})).await,
        field("code", "required")
    );
    let (s, j) = post(
        &app,
        &cookie,
        "/api/account/mfa/enable",
        enable(code_now(&secret)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{j}");
    let codes = j["recoveryCodes"].as_array().expect("codes");
    assert_eq!(codes.len(), 10);
    assert!(
        codes
            .iter()
            .all(|c| c.as_str().is_some_and(|c| c.len() == 11))
    );

    assert_eq!(me(&cookie).await["user"]["mfaEnabled"], json!(true));
    let (s, j) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/account/mfa",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(
        (s, j),
        (
            StatusCode::OK,
            json!({ "enabled": true, "recoveryCodesLeft": 10, "requiredBy": [] })
        )
    );
    let conflict = (StatusCode::CONFLICT, json!({ "code": "mfa_enabled" }));
    assert_eq!(
        post(&app, &cookie, "/api/account/mfa/setup", setup).await,
        conflict
    );
    assert_eq!(
        post(
            &app,
            &cookie,
            "/api/account/mfa/enable",
            enable(code_now(&secret))
        )
        .await,
        conflict
    );
    // Stored sealed, never in plain text.
    let row = db
        .conn
        .query_one(sea_orm::Statement::from_string(
            db.conn.get_database_backend(),
            "SELECT totp_secret FROM users WHERE email = 'owner@example.com'",
        ))
        .await
        .expect("query")
        .expect("row");
    let sealed: Vec<u8> = row.try_get_by_index(0).expect("bytes");
    assert_eq!(sealed.len(), 12 + 20 + 16);
    assert!(!sealed.windows(20).any(|w| w == secret.as_slice()));
}

#[tokio::test]
async fn expired_or_missing_pending_secret() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let cookie = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let expired = field("code", "expired");
    let body = json!({ "code": "123456" });
    assert_eq!(
        post(&app, &cookie, "/api/account/mfa/enable", body.clone()).await,
        expired
    );
    let (_, j) = post(
        &app,
        &cookie,
        "/api/account/mfa/setup",
        json!({ "password": OWNER_PASSWORD }),
    )
    .await;
    let secret = base32_decode(j["secret"].as_str().expect("secret"));
    db.conn
        .execute_unprepared(
            "UPDATE users SET totp_pending_expires_at = now() - interval '1 second'",
        )
        .await
        .expect("expire");
    let body = json!({ "code": code_now(&secret) });
    assert_eq!(
        post(&app, &cookie, "/api/account/mfa/enable", body).await,
        expired
    );
}

#[tokio::test]
async fn account_mfa_routes_are_session_only() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let forbidden = (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }));
    let t = As::Bearer(TEST_TOKEN);
    assert_eq!(
        on(&app, Method::GET, TEST_HOST, "/api/account/mfa", t, None).await,
        forbidden
    );
    let body = Some(json!({ "password": OWNER_PASSWORD }));
    assert_eq!(
        on(
            &app,
            Method::POST,
            TEST_HOST,
            "/api/account/mfa/setup",
            t,
            body
        )
        .await,
        forbidden
    );
    // Also on a space host with a session.
    let cookie = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let (s, j) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/account/mfa",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!((s, j["enabled"].clone()), (StatusCode::OK, json!(false)));
}

#[tokio::test]
async fn wrong_codes_count_in_the_login_bucket() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let cookie = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    post(
        &app,
        &cookie,
        "/api/account/mfa/setup",
        json!({ "password": OWNER_PASSWORD }),
    )
    .await;
    for _ in 0..5 {
        let (s, _) = post(
            &app,
            &cookie,
            "/api/account/mfa/enable",
            json!({ "code": "000000x" }),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (s, j) = post(
        &app,
        &cookie,
        "/api/account/mfa/enable",
        json!({ "code": "000000" }),
    )
    .await;
    assert_eq!(
        (s, j),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        )
    );
}

#[tokio::test]
async fn recovery_codes_regenerate() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let path = "/api/account/mfa/recovery-codes";
    let body = |code: &str| json!({ "password": OWNER_PASSWORD, "code": code });
    assert_eq!(
        post(&app, &e.cookie, path, json!({ "password": OWNER_PASSWORD })).await,
        field("code", "required")
    );
    assert_eq!(
        post(&app, &e.cookie, path, body("999999")).await,
        field("code", "invalid")
    );
    // A wrong password is reported; the code is not checked (nor spent).
    let (s, j) = post(
        &app,
        &e.cookie,
        path,
        json!({ "password": "nope", "code": e.codes[0] }),
    )
    .await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "password": "invalid" })
        )
    );
    // A recovery code works as the step-up code and is spent.
    let (s, j) = post(&app, &e.cookie, path, body(&e.codes[0].to_uppercase())).await;
    assert_eq!(s, StatusCode::OK, "{j}");
    let new: Vec<String> = serde_json::from_value(j["recoveryCodes"].clone()).expect("codes");
    assert_eq!(new.len(), 10);
    // The old codes are gone, the new ones work.
    assert_eq!(
        post(&app, &e.cookie, path, body(&e.codes[1])).await,
        field("code", "invalid")
    );
    let (s, _) = post(&app, &e.cookie, path, body(&new[0])).await;
    assert_eq!(s, StatusCode::OK);
    let (_, j) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/account/mfa",
        As::Cookie(&e.cookie),
        None,
    )
    .await;
    assert_eq!(j["recoveryCodesLeft"], json!(10));
}

#[tokio::test]
async fn disable_drops_sessions_and_tokens_in_requiring_spaces() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (beta, _) = common::auth::seed_space(&db.conn, "beta", "beta@example.com", None).await;
    join(&db.conn, beta, db.user_id, Role::Admin).await;
    let e = enrol(&app, OWNER_EMAIL).await;
    require_mfa(&db.conn, "acme").await;
    let acme_session = session_on(&db.conn, db.space, db.user_id).await;
    let beta_session = session_on(&db.conn, beta, db.user_id).await;
    let beta_token = token(&db.conn, beta, db.user_id, Role::Admin).await;

    let (_, j) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/account/mfa",
        As::Cookie(&e.cookie),
        None,
    )
    .await;
    assert_eq!(
        j["requiredBy"],
        json!([{ "slug": "acme", "name": "Space acme" }])
    );

    let path = "/api/account/mfa/disable";
    assert_eq!(
        post(&app, &e.cookie, path, json!({ "password": OWNER_PASSWORD })).await,
        field("code", "required")
    );
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (s, _) = post(
        &app,
        &e.cookie,
        path,
        json!({ "password": OWNER_PASSWORD, "code": code }),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let me = async |host: &str, who: As<'_>| me_status(&app, host, who).await;
    assert_eq!(
        me(TEST_HOST, As::Cookie(&acme_session)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        me(TEST_HOST, As::Bearer(TEST_TOKEN)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(me(B_HOST, As::Cookie(&beta_session)).await, StatusCode::OK);
    assert_eq!(me(B_HOST, As::Bearer(&beta_token)).await, StatusCode::OK);
    assert_eq!(me(BASE_HOST, As::Cookie(&e.cookie)).await, StatusCode::OK);
    assert_eq!(
        count(&db.conn, "SELECT count(*) FROM recovery_codes").await,
        0
    );
    let (_, j) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/account/mfa",
        As::Cookie(&e.cookie),
        None,
    )
    .await;
    assert_eq!(j["enabled"], json!(false));
}
