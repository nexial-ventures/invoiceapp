//! The CSRF Origin check and host-bound session cookies (auth.md, 4a).

mod common;

use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use common::auth::{As, exchange, login, on, request};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb, router};
use sea_orm::ConnectionTrait;
use serde_json::json;

#[tokio::test]
async fn cookies_are_bound_to_their_host() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    common::auth::seed_space(&db.conn, "beta", "beta@example.com", None).await;
    // The owner of acme is also made a member of beta, so only the host differs.
    let space = invoiceapp::space::repo::find_by_slug(&db.conn, "beta")
        .await
        .expect("q")
        .expect("beta");
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO space_members (space_id, user_id, role) VALUES ('{}', '{}', 'member')",
            space.id, db.user_id
        ))
        .await
        .expect("member");
    let acme = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("acme");
    let base = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("base");
    let check = |host: &'static str, c: String| {
        let app = app.clone();
        async move {
            on(
                &app,
                Method::GET,
                host,
                "/api/auth/me",
                As::Cookie(&c),
                None,
            )
            .await
            .0
        }
    };
    assert_eq!(
        check("beta.localhost:3000", acme.clone()).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        check(BASE_HOST, acme.clone()).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        check(TEST_HOST, base.clone()).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(check(TEST_HOST, acme).await, StatusCode::OK);
    assert_eq!(check(BASE_HOST, base).await, StatusCode::OK);
}

#[tokio::test]
async fn csrf_origin_check() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let cookie = login(&app, TEST_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let contact = json!({ "name": "Odběratel s.r.o." });
    let post = |origin: Option<&str>, who: As<'_>| {
        let mut b = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/api/contacts")
            .header(header::HOST, TEST_HOST)
            .header(header::CONTENT_TYPE, "application/json");
        match who {
            As::Cookie(c) => b = b.header(header::COOKIE, format!("invoice_session={c}")),
            As::Bearer(t) => b = b.header(header::AUTHORIZATION, format!("Bearer {t}")),
            As::Nobody => {}
        }
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        b.body(Body::from(contact.to_string())).expect("request")
    };
    let csrf = (StatusCode::FORBIDDEN, json!({ "code": "csrf" }));
    let (s, _, j) = exchange(&app, post(None, As::Cookie(&cookie))).await;
    assert_eq!((s, j), csrf, "cookie POST without Origin");
    let (s, _, j) = exchange(
        &app,
        post(Some("http://evil.example.com"), As::Cookie(&cookie)),
    )
    .await;
    assert_eq!((s, j), csrf, "foreign Origin");
    let (s, _, j) = exchange(
        &app,
        post(Some("http://localhost:3000"), As::Cookie(&cookie)),
    )
    .await;
    assert_eq!((s, j), csrf, "the base host is another origin");
    let (s, _, _) = exchange(
        &app,
        post(Some("http://acme.localhost:3000"), As::Cookie(&cookie)),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "own origin");
    let (s, _, _) = exchange(&app, post(None, As::Bearer(TEST_TOKEN))).await;
    assert_eq!(s, StatusCode::CREATED, "Bearer without Origin");
    let (s, _, j) = exchange(
        &app,
        post(Some("http://evil.example.com"), As::Bearer(TEST_TOKEN)),
    )
    .await;
    assert_eq!((s, j), csrf, "Bearer with a foreign Origin");
    // A cookie GET needs no Origin.
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/contacts",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // A cross-site login form post is refused before any credential check.
    let body = json!({ "email": OWNER_EMAIL, "password": OWNER_PASSWORD });
    let req = request(
        Method::POST,
        TEST_HOST,
        "/api/auth/login",
        As::Nobody,
        Some(body.clone()),
    )
    .header(header::ORIGIN, "http://evil.example.com")
    .body(Body::from(body.to_string()))
    .expect("request");
    let (s, _, j) = exchange(&app, req).await;
    assert_eq!((s, j), csrf);
}
