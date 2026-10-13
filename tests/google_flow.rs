//! Sign in with Google (google.md, 4d): the flow and the handoff are single
//! use, expire, and are bound to their host and browser; `start` checks.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, on};
use common::google::{self, at, get, location, sign_in};
use common::oidc::claims;
use common::{BASE_HOST, OWNER_EMAIL, TEST_HOST, TestDb};
use sea_orm::ConnectionTrait;
use serde_json::json;

#[tokio::test]
async fn state_is_single_use_and_expires() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    assert!(
        to.starts_with(&at(TEST_HOST, "/api/auth/google/finish?token=")),
        "{to}"
    );
    // Reused state: unknown now, the origin is not known any more.
    let again =
        google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    assert_eq!(again, at(BASE_HOST, "/login?google_error=invalid"));
    let unknown = google::callback_raw(&app, "state=nope&code=x").await;
    assert_eq!(unknown, at(BASE_HOST, "/login?google_error=invalid"));
    assert_eq!(
        google::callback_raw(&app, "").await,
        at(BASE_HOST, "/login?google_error=invalid")
    );

    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    db.conn
        .execute_unprepared("UPDATE google_flows SET expires_at = now() - interval '1 second'")
        .await
        .expect("expire");
    let requests = mock.token_requests();
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    assert_eq!(to, at(TEST_HOST, "/login?google_error=expired"));
    assert_eq!(
        mock.token_requests(),
        requests,
        "no code exchange for a dead flow"
    );
}

#[tokio::test]
async fn handoff_is_single_use_host_bound_and_expires() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    let cookies = [st.browser.clone()];
    // Another host: refused, and the handoff is not used up by it.
    let done = google::finish(&app, &to, BASE_HOST, &cookies).await;
    assert_eq!(done.location, at(BASE_HOST, "/login?google_error=invalid"));
    assert!(done.session.is_none());
    let done = google::finish(&app, &to, TEST_HOST, &cookies).await;
    assert!(done.session.is_some(), "{}", done.location);
    let done = google::finish(&app, &to, TEST_HOST, &cookies).await;
    assert_eq!(
        done.location,
        at(TEST_HOST, "/login?google_error=invalid"),
        "reused"
    );

    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    db.conn
        .execute_unprepared("UPDATE google_handoffs SET expires_at = now() - interval '1 second'")
        .await
        .expect("expire");
    let done = google::finish(&app, &to, TEST_HOST, &[st.browser]).await;
    assert_eq!(done.location, at(TEST_HOST, "/login?google_error=expired"));
    assert!(done.session.is_none());
}

/// A callback URL of someone else's flow (login CSRF) does not sign the
/// victim's browser in: it lacks the starting browser's cookie.
#[tokio::test]
async fn handoff_needs_the_starting_browser() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    // No cookie, a forged value, or another flow's binding cookie.
    let other = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let name = st.browser.split('=').next().expect("name").to_string();
    for cookies in [
        vec![],
        vec![format!("{name}=someone-else")],
        vec![other.browser.clone()],
    ] {
        let done = google::finish(&app, &to, TEST_HOST, &cookies).await;
        assert_eq!(done.location, at(TEST_HOST, "/login?google_error=invalid"));
        assert!(done.session.is_none());
    }
    // The cookie is cleared when the handoff ends.
    let st = google::start(&app, &mock, TEST_HOST, "purpose=login", &[]).await;
    let to = google::callback(&app, &mock, &st, claims(&mock, "s", OWNER_EMAIL, &st.nonce)).await;
    let path = to
        .split_once("3000")
        .map(|(_, p)| p.to_string())
        .expect("path");
    let name = st.browser.split('=').next().expect("name").to_string();
    let (s, h, _) = get(&app, TEST_HOST, &path, &[st.browser, other.browser]).await;
    assert_eq!(s, StatusCode::FOUND);
    let cleared: Vec<&str> = h
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter(|v| v.starts_with("invoice_google_") && v.contains("Max-Age=0"))
        .collect();
    assert_eq!(cleared.len(), 1, "only its own binding is cleared");
    assert!(cleared[0].starts_with(&format!("{name}=;")));
    assert_eq!(h["cache-control"], "no-store");
}

#[tokio::test]
async fn start_checks() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    let (s, h, _) = get(
        &app,
        TEST_HOST,
        "/api/auth/google/start?purpose=register",
        &[],
    )
    .await;
    assert_eq!(
        (s, location(&h)),
        (
            StatusCode::FOUND,
            at(TEST_HOST, "/login?google_error=invalid")
        )
    );
    // The redirect keeps its own referrer policy under the global headers.
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h.get_all("referrer-policy").iter().count(), 1);
    assert_eq!(h["x-frame-options"], "DENY");
    // An invitation on the base host or with a dead token.
    let (_, h, _) = get(
        &app,
        BASE_HOST,
        "/api/auth/google/start?purpose=invite&invite=abc",
        &[],
    )
    .await;
    assert_eq!(
        location(&h),
        at(BASE_HOST, "/invite?token=abc&google_error=invalid")
    );
    let (_, h, _) = get(
        &app,
        TEST_HOST,
        "/api/auth/google/start?purpose=invite&invite=dead",
        &[],
    )
    .await;
    assert_eq!(
        location(&h),
        at(TEST_HOST, "/invite?token=dead&google_error=invalid")
    );
    // `link` needs the grant of `POST /api/account/google/link`.
    let base = common::auth::login(&app, BASE_HOST, OWNER_EMAIL, common::OWNER_PASSWORD)
        .await
        .expect("login");
    let cookie = vec![format!("invoice_session={base}")];
    let (s, h, _) = get(
        &app,
        BASE_HOST,
        "/api/auth/google/start?purpose=link",
        &cookie,
    )
    .await;
    assert_eq!(
        (s, location(&h)),
        (
            StatusCode::FOUND,
            at(BASE_HOST, "/account?google_error=invalid")
        )
    );
    // 20 starts per IP and 15 minutes (4 above, every request counts).
    for _ in 0..16 {
        google::start(&app, &mock, BASE_HOST, "purpose=login", &[]).await;
    }
    let (s, h, j) = get(&app, BASE_HOST, "/api/auth/google/start?purpose=login", &[]).await;
    assert_eq!(
        (s, j["code"].clone()),
        (StatusCode::TOO_MANY_REQUESTS, json!("rate_limited"))
    );
    assert!(h.contains_key("retry-after"));
}

#[tokio::test]
async fn provider_unreachable() {
    let db = TestDb::new().await;
    let (app, mock) = google::app(&db.conn, false);
    // A provider that is not there: discovery fails at the start.
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    };
    let dead_app = common::TestApp::builder(&db.conn)
        .mdcast_url("http://127.0.0.1:9")
        .google(&format!("http://127.0.0.1:{dead}"))
        .build();
    let (s, h, _) = get(
        &dead_app,
        TEST_HOST,
        "/api/auth/google/start?purpose=login",
        &[],
    )
    .await;
    assert_eq!(
        (s, location(&h)),
        (
            StatusCode::FOUND,
            at(TEST_HOST, "/login?google_error=unavailable")
        )
    );
    // The working app is unaffected.
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
    assert!(done.session.is_some());
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Cookie(&done.session.unwrap_or_default()),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}
