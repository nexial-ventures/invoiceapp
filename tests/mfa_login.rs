//! The login's code step (mfa.md, 4c): TOTP, recovery code, wrong and
//! replayed codes, the pending cookie (host, expiry, single use, failures)
//! and the login bucket.

mod common;

use axum::http::{Method, StatusCode, header};
use common::auth::{As, login, on};
use common::mfa::{enrol, finish, fresh, start};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TestDb, router};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

fn invalid() -> Value {
    json!({ "code": "invalid_credentials" })
}

fn mfa_invalid() -> (StatusCode, Value) {
    (StatusCode::UNAUTHORIZED, json!({ "code": "mfa_invalid" }))
}

#[tokio::test]
async fn totp_login_on_a_space_host() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (s, cookie, _, headers) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let cookie = cookie.expect("session cookie");
    let cleared = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.starts_with("invoice_mfa=;") && v.contains("Max-Age=0"));
    assert!(cleared, "{headers:?}");
    let (s, me) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(
        (s, me["user"]["mfaEnabled"].clone()),
        (StatusCode::OK, json!(true))
    );
    // Single use.
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!((s, j), (StatusCode::UNAUTHORIZED, invalid()));
}

#[tokio::test]
async fn recovery_code_login_works_once() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let pending = start(&app, BASE_HOST, OWNER_EMAIL).await;
    let typed = format!(" {} ", e.codes[3].to_uppercase());
    let (s, cookie, _, _) = finish(&app, BASE_HOST, Some(&pending), &typed).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(cookie.is_some());
    let pending = start(&app, BASE_HOST, OWNER_EMAIL).await;
    let (s, _, j, _) = finish(&app, BASE_HOST, Some(&pending), &e.codes[3]).await;
    assert_eq!((s, j), mfa_invalid());
    let (_, j) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/account/mfa",
        As::Cookie(&e.cookie),
        None,
    )
    .await;
    assert_eq!(j["recoveryCodesLeft"], json!(9));
}

#[tokio::test]
async fn wrong_and_replayed_codes() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    for wrong in ["000000x", "abc", ""] {
        let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), wrong).await;
        assert_eq!((s, j), mfa_invalid(), "{wrong:?}");
    }
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (s, _, _, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // The same code again (the step is used up) is refused.
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!((s, j), mfa_invalid());
}

#[tokio::test]
async fn pending_login_is_bound_to_host_and_time() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    // No cookie.
    let (s, _, j, _) = finish(&app, TEST_HOST, None, &code).await;
    assert_eq!((s, j), (StatusCode::UNAUTHORIZED, invalid()));
    // A base-host pending login on a space host.
    let pending = start(&app, BASE_HOST, OWNER_EMAIL).await;
    let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!((s, j), (StatusCode::UNAUTHORIZED, invalid()));
    // Expired.
    db.conn
        .execute_unprepared("UPDATE mfa_logins SET expires_at = now() - interval '1 second'")
        .await
        .expect("expire");
    let (s, _, j, _) = finish(&app, BASE_HOST, Some(&pending), &code).await;
    assert_eq!((s, j), (StatusCode::UNAUTHORIZED, invalid()));
}

#[tokio::test]
async fn pending_login_dies_after_five_failures_and_the_code_bucket_fills() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    for _ in 0..5 {
        let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), "111111").await;
        assert_eq!((s, j), mfa_invalid());
    }
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!((s, j), (StatusCode::UNAUTHORIZED, invalid()));
    // The five wrong codes filled the user's (hard) code bucket: the
    // password still passes, the code step of a new pending login is 429.
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let code = fresh(&db.conn, db.user_id, &e.secret).await;
    let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
    assert_eq!(
        (s, j),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        )
    );
}

#[tokio::test]
async fn users_without_totp_log_in_as_before() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let cookie = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD).await;
    assert!(cookie.is_ok());
    let (s, me) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&cookie.expect("c")),
        None,
    )
    .await;
    assert_eq!(
        (s, me["user"]["mfaEnabled"].clone()),
        (StatusCode::OK, json!(false))
    );
}
