//! Removal, leaving and role changes take effect on sessions and tokens at
//! once (members.md, 4b).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, member, on, seed_space};
use common::members::{B_HOST, join, session_on, token, user_id};
use common::{TEST_HOST, TEST_TOKEN, TestDb, router};
use invoiceapp::space::Role;
use serde_json::{Value, json};

async fn put_role(
    app: &axum::Router,
    who: As<'_>,
    user: uuid::Uuid,
    role: &str,
) -> (StatusCode, Value) {
    let uri = format!("/api/members/{user}");
    on(
        app,
        Method::PUT,
        TEST_HOST,
        &uri,
        who,
        Some(json!({ "role": role })),
    )
    .await
}

async fn remove(app: &axum::Router, who: As<'_>, user: uuid::Uuid) -> (StatusCode, Value) {
    let uri = format!("/api/members/{user}");
    on(app, Method::DELETE, TEST_HOST, &uri, who, None).await
}

#[tokio::test]
async fn removal_and_leave_revoke_access_in_this_space_only() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", None).await;
    let x_token = member(&db.conn, db.space, "x@example.com", Role::Member).await;
    let x = user_id(&db.conn, "x@example.com").await;
    join(&db.conn, beta, x, Role::Member).await;
    let x_beta_token = token(&db.conn, beta, x, Role::Member).await;
    let x_cookie = session_on(&db.conn, db.space, x).await;
    let x_beta_cookie = session_on(&db.conn, beta, x).await;
    let me = |host: &'static str, who| {
        let app = app.clone();
        async move { on(&app, Method::GET, host, "/api/space", who, None).await.0 }
    };
    assert_eq!(me(TEST_HOST, As::Cookie(&x_cookie)).await, StatusCode::OK);

    assert_eq!(
        remove(&app, As::Bearer(TEST_TOKEN), x).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        me(TEST_HOST, As::Bearer(&x_token)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        me(TEST_HOST, As::Cookie(&x_cookie)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(me(B_HOST, As::Bearer(&x_beta_token)).await, StatusCode::OK);
    assert_eq!(me(B_HOST, As::Cookie(&x_beta_cookie)).await, StatusCode::OK);
    let rows = common::members::count(
        &db.conn,
        &format!(
            "SELECT (SELECT count(*) FROM sessions WHERE user_id = '{x}' AND space_id = '{}') \
                  + (SELECT count(*) FROM api_tokens WHERE user_id = '{x}' AND space_id = '{}')",
            db.space, db.space
        ),
    )
    .await;
    assert_eq!(rows, 0, "rows deleted, not just refused");

    // Leaving: session only, the cookie is cleared, tokens die with it.
    let y_token = member(&db.conn, db.space, "y@example.com", Role::Accountant).await;
    let y = user_id(&db.conn, "y@example.com").await;
    join(&db.conn, beta, y, Role::Admin).await;
    let y_beta_cookie = session_on(&db.conn, beta, y).await;
    let y_cookie = session_on(&db.conn, db.space, y).await;
    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/space/leave",
        As::Bearer(&y_token),
        None,
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }))
    );
    let req = common::auth::request(
        Method::POST,
        TEST_HOST,
        "/api/space/leave",
        As::Cookie(&y_cookie),
        None,
    )
    .body(axum::body::Body::empty())
    .expect("request");
    let (s, headers, _) = common::auth::exchange(&app, req).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let set = headers
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        set.starts_with("invoice_session=;") && set.contains("Max-Age=0"),
        "{set}"
    );
    assert_eq!(
        me(TEST_HOST, As::Cookie(&y_cookie)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        me(TEST_HOST, As::Bearer(&y_token)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(me(B_HOST, As::Cookie(&y_beta_cookie)).await, StatusCode::OK);
    // A cookie mutation needs the own Origin (CSRF) like any other.
    let z_cookie = session_on(&db.conn, beta, y).await;
    let req = common::auth::request(Method::POST, B_HOST, "/api/space/leave", As::Nobody, None)
        .header("cookie", format!("invoice_session={z_cookie}"))
        .header("origin", "http://evil.example.com")
        .body(axum::body::Body::empty())
        .expect("request");
    let (s, _, j) = common::auth::exchange(&app, req).await;
    assert_eq!((s, j), (StatusCode::FORBIDDEN, json!({ "code": "csrf" })));
}

#[tokio::test]
async fn a_role_change_keeps_sessions_and_caps_tokens() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let admin_token = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let admin = user_id(&db.conn, "admin@example.com").await;
    let cookie = session_on(&db.conn, db.space, admin).await;
    let rename = Some(json!({ "name": "Acme" }));
    let (s, _) = on(
        &app,
        Method::PUT,
        TEST_HOST,
        "/api/space",
        As::Bearer(&admin_token),
        rename.clone(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    assert_eq!(
        put_role(&app, As::Bearer(TEST_TOKEN), admin, "accountant")
            .await
            .0,
        StatusCode::OK
    );
    let (s, _) = on(
        &app,
        Method::PUT,
        TEST_HOST,
        "/api/space",
        As::Bearer(&admin_token),
        rename,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "token capped by the new role");
    let (s, _) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/space",
        As::Bearer(&admin_token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "the token stays");
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
        (s, me["space"]["role"].clone()),
        (StatusCode::OK, json!("accountant"))
    );
}
