//! Registration, e-mail verification and the first space (auth.md, 4a).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{
    As, PASSWORD, login, mail, mail_app, mail_text, mail_token, on, register_body, settle,
};
use common::smtp::EmailEnv;
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TestDb};
use serde_json::json;

#[tokio::test]
async fn register_verify_login_create_space_and_log_in_on_its_host() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, true);

    let (status, ctx) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/context",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        ctx,
        json!({ "kind": "base", "space": null, "registration": true, "baseUrl": "http://localhost:3000", "google": false })
    );

    let body = register_body(" Jana@Example.com ");
    let (status, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let m = mail(&env.smtp, 1).await;
    assert_eq!(m.rcpt, ["<jana@example.com>"]);
    assert!(mail_text(&m).contains("http://localhost:3000/verify?token="));
    let token = mail_token(&m, "verify");

    // Unverified: may log in on the base host, may not create a space.
    let cookie = login(&app, BASE_HOST, "jana@example.com", PASSWORD)
        .await
        .expect("base login while unverified");
    let space = json!({ "slug": "jana", "name": "Jana s.r.o." });
    let (status, body) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&cookie),
        Some(space.clone()),
    )
    .await;
    assert_eq!(
        (status, &body["code"]),
        (StatusCode::FORBIDDEN, &json!("email_unverified"))
    );
    let (_, me) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/auth/me",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(me["user"]["emailVerified"], false);
    assert_eq!(me["space"], json!(null));

    let (status, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify",
        As::Nobody,
        Some(json!({ "token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/verify",
        As::Nobody,
        Some(json!({ "token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "single use");
    assert_eq!(body["fields"], json!({ "token": "invalid" }));

    let (status, created) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&cookie),
        Some(space),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created,
        json!({ "slug": "jana", "name": "Jana s.r.o.", "role": "owner", "url": "http://jana.localhost:3000", "requireMfa": false })
    );
    let (_, list) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(list, json!([created]));

    // The base-host session is not valid on the space host: log in there.
    let host = "jana.localhost:3000";
    let (status, _) = on(
        &app,
        Method::GET,
        host,
        "/api/space",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let space_cookie = login(&app, host, "jana@example.com", PASSWORD)
        .await
        .expect("space login");
    let (status, me) = on(
        &app,
        Method::GET,
        host,
        "/api/auth/me",
        As::Cookie(&space_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        me["space"],
        json!({ "slug": "jana", "name": "Jana s.r.o.", "role": "owner" })
    );
    assert_eq!(me["user"]["displayName"], "Jana Nováková");
    let (status, _) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&space_cookie),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a space cookie on the base host"
    );

    // The new space got every seed of a fresh instance.
    let (_, rates) = on(
        &app,
        Method::GET,
        host,
        "/api/settings/vat-rates",
        As::Cookie(&space_cookie),
        None,
    )
    .await;
    assert_eq!(rates.as_array().map(Vec::len), Some(3));
    let (_, series) = on(
        &app,
        Method::GET,
        host,
        "/api/settings/number-series",
        As::Cookie(&space_cookie),
        None,
    )
    .await;
    assert_eq!(series.as_array().map(Vec::len), Some(14));
    let (status, _) = on(
        &app,
        Method::GET,
        host,
        "/api/settings/company",
        As::Cookie(&space_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, acc) = on(
        &app,
        Method::GET,
        host,
        "/api/settings/accounting",
        As::Cookie(&space_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{acc}");
}

#[tokio::test]
async fn registration_is_enumeration_safe_and_validated() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, true);

    // A known e-mail: the same 202, an "already registered" mail with a reset link.
    let (status, _) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body(OWNER_EMAIL)),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let m = mail(&env.smtp, 1).await;
    assert_eq!(m.rcpt, [format!("<{OWNER_EMAIL}>")]);
    mail_token(&m, "reset");
    assert!(
        login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
            .await
            .is_ok(),
        "account unchanged"
    );

    let (status, body) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(json!({ "email": "nope", "password": "short", "displayName": "", "locale": "en" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["fields"],
        json!({ "email": "invalid", "password": "too_short", "displayName": "required" })
    );

    // English mails on request.
    let body = json!({ "email": "eve@example.com", "password": PASSWORD, "displayName": "Eve", "locale": "en" });
    on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(body),
    )
    .await;
    assert!(mail_text(&mail(&env.smtp, 2).await).contains("Hello"));
}

#[tokio::test]
async fn registration_switch_and_hosts() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let off = mail_app(&db.conn, &env, false);
    let (status, body) = on(
        &off,
        Method::POST,
        BASE_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body("a@example.com")),
    )
    .await;
    assert_eq!(
        (status, body),
        (StatusCode::NOT_FOUND, json!({ "code": "not_found" }))
    );
    let (_, ctx) = on(
        &off,
        Method::GET,
        BASE_HOST,
        "/api/context",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(ctx["registration"], false);

    let on_app = mail_app(&db.conn, &env, true);
    let (status, _) = on(
        &on_app,
        Method::POST,
        common::TEST_HOST,
        "/api/auth/register",
        As::Nobody,
        Some(register_body("a@example.com")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "registration lives on the base host only"
    );
    settle().await;
    assert!(env.smtp.mails().is_empty());
}
