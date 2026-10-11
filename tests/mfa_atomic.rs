//! Codes are never spent by a request that fails, the pending login's
//! failure cap holds under concurrency, and a changed secret key leaves the
//! recovery codes usable (mfa.md, 4c).

mod common;

use axum::http::StatusCode;
use common::auth::{member, user};
use common::members::{accept, count, invited};
use common::mfa::{enrol, finish, fresh, start};
use common::{OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TestDb, router};
use invoice::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::json;

async fn codes_left(db: &TestDb, email: &str) -> i64 {
    let sql = format!(
        "SELECT count(*) FROM recovery_codes r JOIN users u ON u.id = r.user_id \
         WHERE u.email = '{email}'"
    );
    count(&db.conn, &sql).await
}

#[tokio::test]
async fn concurrent_wrong_codes_hit_the_cap_exactly() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    enrol(&app, OWNER_EMAIL).await;
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let tries = (0..8).map(|_| finish(&app, TEST_HOST, Some(&pending), "111111"));
    let results = futures_util::future::join_all(tries).await;
    let invalid = results
        .iter()
        .filter(|r| r.2 == json!({ "code": "mfa_invalid" }))
        .count();
    let gone = results
        .iter()
        .filter(|r| r.2 == json!({ "code": "invalid_credentials" }))
        .count();
    assert_eq!(
        (invalid, gone),
        (5, 3),
        "{:?}",
        results.iter().map(|r| &r.2).collect::<Vec<_>>()
    );
    assert_eq!(count(&db.conn, "SELECT count(*) FROM mfa_logins").await, 0);
}

#[tokio::test]
async fn parallel_code_steps_spend_one_code() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let (a, b) = tokio::join!(
        finish(&app, TEST_HOST, Some(&pending), &e.codes[0]),
        finish(&app, TEST_HOST, Some(&pending), &e.codes[1]),
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::NO_CONTENT, StatusCode::UNAUTHORIZED]);
    assert_eq!(codes_left(&db, OWNER_EMAIL).await, 9);
}

#[tokio::test]
async fn a_refused_accept_does_not_spend_the_code() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    user(&db.conn, "jana@example.com").await;
    let e = enrol(&app, "jana@example.com").await;
    let (s, j) = common::members::invite(
        &app,
        common::auth::As::Bearer(&admin),
        "jana@example.com",
        "member",
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    let token = common::members::link_token(&j["url"]);
    // The inviter loses the right to grant (written directly, so the
    // invitation survives until the accept re-checks it).
    db.conn
        .execute_unprepared(
            "UPDATE space_members SET role = 'accountant' WHERE user_id = \
             (SELECT id FROM users WHERE email = 'admin@example.com')",
        )
        .await
        .expect("demote");
    let body = json!({ "token": token, "password": OWNER_PASSWORD, "code": e.codes[0] });
    let (s, _, j, _) = accept(&app, TEST_HOST, body).await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "token": "invalid" })
        )
    );
    assert_eq!(codes_left(&db, "jana@example.com").await, 10);

    // A wrong code rolls the whole accept back: the invitation stays usable.
    let token = invited(&app, "jana@example.com", "member").await;
    let wrong = json!({ "token": token, "password": OWNER_PASSWORD, "code": "000000" });
    let (s, _, j, _) = accept(&app, TEST_HOST, wrong).await;
    assert_eq!(
        (s, j["fields"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "code": "invalid" })
        )
    );
    let uid = common::members::user_id(&db.conn, "jana@example.com").await;
    let code = fresh(&db.conn, uid, &e.secret).await;
    let ok = json!({ "token": token, "password": OWNER_PASSWORD, "code": code });
    assert_eq!(accept(&app, TEST_HOST, ok).await.0, StatusCode::NO_CONTENT);
}

/// A TOTP secret that no longer decrypts (here: a damaged row; in practice
/// also a changed `INVOICE__SECRET_KEY`) answers TOTP codes as wrong without
/// counting them and without a 500, so recovery codes stay reachable.
#[tokio::test]
async fn an_undecryptable_secret_keeps_recovery_codes_usable() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let e = enrol(&app, OWNER_EMAIL).await;
    db.conn
        .execute_unprepared(
            // XOR, not overwrite: an overwrite with 0xff is a no-op when the
            // random ciphertext already has 0xff there (1 in 256 runs).
            "UPDATE users SET totp_secret = set_byte(totp_secret, 19, get_byte(totp_secret, 19) # 255) \
             WHERE email = 'owner@example.com'",
        )
        .await
        .expect("damage the secret");
    // More TOTP attempts than the login bucket (5) allows: none of them count.
    for _ in 0..2 {
        let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
        for _ in 0..4 {
            let code = fresh(&db.conn, db.user_id, &e.secret).await;
            let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
            assert_eq!(
                (s, j),
                (StatusCode::UNAUTHORIZED, json!({ "code": "mfa_invalid" }))
            );
        }
    }
    let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
    let (s, cookie, _, _) = finish(&app, TEST_HOST, Some(&pending), &e.codes[0]).await;
    assert_eq!((s, cookie.is_some()), (StatusCode::NO_CONTENT, true));
}

/// Another `INVOICE__SECRET_KEY`: TOTP codes are wrong (no 500) and never
/// fill the login bucket — the password step keeps working. (Recovery codes
/// are HMAC-keyed by the same key, so they stop matching too.)
#[tokio::test]
async fn a_changed_secret_key_does_not_lock_out_the_login_bucket() {
    let db = TestDb::new().await;
    let e = enrol(&router(db.conn.clone()), OWNER_EMAIL).await;
    let mut state = common::state(
        db.conn.clone(),
        invoice::ares::DEFAULT_ARES_URL,
        invoice::cnb::DEFAULT_CNB_URL,
        common::pdf_service("http://127.0.0.1:9", common::shared_storage()),
    );
    state.secret_key = invoice::secret::SecretKey::from_bytes([7; 32]);
    let app = common::app(state);
    for _ in 0..2 {
        let pending = start(&app, TEST_HOST, OWNER_EMAIL).await;
        for _ in 0..4 {
            let code = fresh(&db.conn, db.user_id, &e.secret).await;
            let (s, _, j, _) = finish(&app, TEST_HOST, Some(&pending), &code).await;
            assert_eq!(
                (s, j),
                (StatusCode::UNAUTHORIZED, json!({ "code": "mfa_invalid" }))
            );
        }
    }
    // Eight refused codes, yet the e-mail's bucket (5) is untouched.
    start(&app, TEST_HOST, OWNER_EMAIL).await;
}
