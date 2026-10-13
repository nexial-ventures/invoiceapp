//! `INVOICEAPP__REGISTRATION` also gates `POST /api/spaces`: with it off only
//! invited people enter the instance (spaces.md, members.md "Accepted risks").

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, on};
use common::members::{accept, invited};
use common::{BASE_HOST, TEST_HOST, TestDb, router, router_open};
use serde_json::json;

const EMAIL: &str = "pozvany@example.com";
const PASSWORD: &str = "an invited user's password";

async fn create(
    app: &axum::Router,
    cookie: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    on(
        app,
        Method::POST,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(cookie),
        Some(body),
    )
    .await
}

#[tokio::test]
async fn an_invited_user_cannot_create_a_space_while_registration_is_off() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let token = invited(&app, EMAIL, "member").await;
    let body = json!({ "token": token, "password": PASSWORD, "displayName": "Pozvaný" });
    let (s, cookie, j, _) = accept(&app, TEST_HOST, body).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    assert!(cookie.is_some());

    // Logs in on its space and on the base host like any account...
    login(&app, TEST_HOST, EMAIL, PASSWORD)
        .await
        .expect("space login");
    let base = login(&app, BASE_HOST, EMAIL, PASSWORD)
        .await
        .expect("base login");
    // ...but may not create a space — refused before the body is validated.
    for body in [json!({ "slug": "vlastni", "name": "Vlastní" }), json!({})] {
        let (s, j) = create(&app, &base, body).await;
        assert_eq!(
            (s, j),
            (
                StatusCode::FORBIDDEN,
                json!({ "code": "space_creation_disabled" })
            )
        );
    }
    let (_, list) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&base),
        None,
    )
    .await;
    assert_eq!(list.as_array().map(Vec::len), Some(1), "{list}");

    // The same account with registration on.
    let app = router_open(db.conn.clone());
    let base = login(&app, BASE_HOST, EMAIL, PASSWORD)
        .await
        .expect("base login");
    let (s, j) = create(&app, &base, json!({ "slug": "vlastni", "name": "Vlastní" })).await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    assert_eq!(
        (&j["slug"], &j["role"]),
        (&json!("vlastni"), &json!("owner"))
    );
}
