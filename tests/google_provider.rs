//! Sign in with Google (google.md, 4d): provider answers — consent denied,
//! token endpoint failures, bad ID tokens, JWKS caching and key rotation.

mod common;

use common::google::{self, at, sign_in};
use common::oidc::claims;
use common::{OWNER_EMAIL, TEST_HOST, TestDb};
use serde_json::json;

#[tokio::test]
async fn provider_answers() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    // The user denied consent.
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback_raw(&app, &format!("state={}&error=access_denied", st.state)).await;
    assert_eq!(to, at(TEST_HOST, "/login?google_error=cancelled"));
    // Token endpoint down / refusing.
    for (status, code) in [(500, "unavailable"), (400, "invalid")] {
        mock.fail_token(Some(status));
        let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
        let to =
            google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
        assert_eq!(
            to,
            at(TEST_HOST, &format!("/login?google_error={code}")),
            "{status}"
        );
    }
    mock.fail_token(None);
    // A code the provider does not know (PKCE / replay).
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback_raw(&app, &format!("state={}&code=forged", st.state)).await;
    assert_eq!(to, at(TEST_HOST, "/login?google_error=invalid"));
    // Wrong nonce, wrong audience, wrong signing key.
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, "other")).await;
    assert_eq!(to, at(TEST_HOST, "/login?google_error=invalid"));
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let mut c = claims(&mock, "s", OWNER_EMAIL, &st.nonce);
    c["aud"] = json!("someone-else");
    assert_eq!(
        google::callback(&app, &mock, &st, c).await,
        at(TEST_HOST, "/login?google_error=invalid")
    );
    mock.sign_with(common::oidc::KEY_B, "test-a");
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    assert_eq!(to, at(TEST_HOST, "/login?google_error=invalid"));
    // Key B appears within the minute after the last JWKS fetch: no
    // refetch, refused.
    mock.sign_with(common::oidc::KEY_B, "test-b");
    mock.publish_b(true);
    let fetched = mock.jwks_fetches();
    assert_eq!(fetched, 1, "discovery + JWKS are cached");
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "s",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(done.location, at(TEST_HOST, "/login?google_error=invalid"));
    assert_eq!(mock.jwks_fetches(), fetched);
}

#[tokio::test]
async fn key_rotation_refetches_the_jwks() {
    let db = TestDb::new().await;
    let (app, mock) = google::app_with(&db.conn, false, std::time::Duration::ZERO);
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "s",
        OWNER_EMAIL,
    )
    .await;
    assert!(done.session.is_some(), "{}", done.location);
    assert_eq!(mock.jwks_fetches(), 1);
    // Known kid: from the cache.
    sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "s",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(mock.jwks_fetches(), 1);
    // A new key: one refetch, accepted.
    mock.sign_with(common::oidc::KEY_B, "test-b");
    mock.publish_b(true);
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "s",
        OWNER_EMAIL,
    )
    .await;
    assert!(done.session.is_some(), "{}", done.location);
    assert_eq!(mock.jwks_fetches(), 2);
    // A kid nobody publishes: one refetch, refused.
    mock.sign_with(common::oidc::KEY_A, "test-c");
    let done = sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "s",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(done.location, at(TEST_HOST, "/login?google_error=invalid"));
    assert_eq!(mock.jwks_fetches(), 3);
}

#[tokio::test]
async fn jwks_expires_by_max_age() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    mock.jwks_cache_control(Some("public, max-age=0"));
    for n in 1..=2 {
        let done = sign_in(
            &app,
            &mock,
            TEST_HOST,
            "purpose=login",
            None,
            "s",
            OWNER_EMAIL,
        )
        .await;
        assert!(done.session.is_some(), "{}", done.location);
        assert_eq!(mock.jwks_fetches(), n, "expired at once: refetched");
    }
    mock.jwks_cache_control(Some("public, max-age=3600"));
    sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "s",
        OWNER_EMAIL,
    )
    .await;
    sign_in(
        &app,
        &mock,
        TEST_HOST,
        "purpose=login",
        None,
        "s",
        OWNER_EMAIL,
    )
    .await;
    assert_eq!(mock.jwks_fetches(), 3, "cached for its max-age");
}

/// Several audiences need `azp` = our client id.
#[tokio::test]
async fn authorized_party() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let mut c = claims(&mock, "s", OWNER_EMAIL, &st.nonce);
    c["aud"] = json!([common::oidc::CLIENT_ID, "another-client"]);
    let to = google::callback(&app, &mock, &st, c).await;
    assert_eq!(to, at(TEST_HOST, "/login?google_error=invalid"));
}
