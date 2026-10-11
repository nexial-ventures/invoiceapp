//! Hosts, `GET /api/context`, the space routes and space delete (spaces.md, 4a).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, login, member, on, seed_space};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TEST_HOST, TEST_TOKEN, TestDb, router};
use invoice::space::Role;
use serde_json::json;

const NOT_FOUND: (StatusCode, &str) = (StatusCode::NOT_FOUND, "not_found");

async fn code(
    app: &axum::Router,
    m: Method,
    host: &str,
    uri: &str,
    who: As<'_>,
) -> (StatusCode, String) {
    let (s, j) = on(app, m, host, uri, who, None).await;
    (s, j["code"].as_str().unwrap_or_default().to_string())
}

#[tokio::test]
async fn hosts_are_classified() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (s, ctx) = on(
        &app,
        Method::GET,
        "ACME.localhost:3000",
        "/api/context",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        ctx,
        json!({ "kind": "space", "space": { "slug": "acme", "name": "Space acme" },
                "registration": false, "baseUrl": "http://localhost:3000", "google": false })
    );
    for host in [
        "nope.localhost:3000",
        "a.acme.localhost:3000",
        "example.com",
        "www.localhost",
        "[::1]:3000",
    ] {
        let (s, j) = on(&app, Method::GET, host, "/api/context", As::Nobody, None).await;
        assert_eq!(
            (s, j),
            (StatusCode::NOT_FOUND, json!({ "code": "not_found" })),
            "{host}"
        );
        let (s, _) = code(
            &app,
            Method::GET,
            host,
            "/api/contacts",
            As::Bearer(TEST_TOKEN),
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{host}");
    }
    // Health and the OpenAPI document answer on any host, without auth.
    for host in ["example.com", BASE_HOST, TEST_HOST] {
        for uri in ["/api/health", "/api/openapi.json"] {
            let (s, _) = on(&app, Method::GET, host, uri, As::Nobody, None).await;
            assert_eq!(s, StatusCode::OK, "{host}{uri}");
        }
    }
    // Business routes do not exist on the base host (404 before auth).
    let base = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("base login");
    for who in [As::Nobody, As::Cookie(&base), As::Bearer(TEST_TOKEN)] {
        let (s, c) = code(&app, Method::GET, BASE_HOST, "/api/contacts", who).await;
        assert_eq!((s, c.as_str()), NOT_FOUND);
        let (s, _) = code(&app, Method::POST, BASE_HOST, "/api/mcp", who).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }
    // `/api/spaces` is a base-host route.
    let (s, _) = code(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/spaces",
        As::Bearer(TEST_TOKEN),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tokens_work_only_on_their_space_host() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    seed_space(&db.conn, "beta", "beta@example.com", None).await;
    let unauthorized = (StatusCode::UNAUTHORIZED, "unauthorized".to_string());
    assert_eq!(
        code(
            &app,
            Method::GET,
            "beta.localhost:3000",
            "/api/contacts",
            As::Bearer(TEST_TOKEN)
        )
        .await,
        unauthorized
    );
    assert_eq!(
        code(
            &app,
            Method::GET,
            BASE_HOST,
            "/api/auth/me",
            As::Bearer(TEST_TOKEN)
        )
        .await,
        unauthorized
    );
    assert_eq!(
        code(
            &app,
            Method::GET,
            TEST_HOST,
            "/api/contacts",
            As::Bearer("inv_00000000_unknown")
        )
        .await,
        unauthorized
    );
    assert_eq!(
        code(
            &app,
            Method::GET,
            TEST_HOST,
            "/api/contacts",
            As::Bearer("not-a-token")
        )
        .await,
        unauthorized
    );
    let (s, _) = code(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/contacts",
        As::Bearer(TEST_TOKEN),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn space_create_rules() {
    let db = TestDb::new().await;
    let app = common::router_open(db.conn.clone());
    let cookie = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let create = |slug: &str, name: &str| json!({ "slug": slug, "name": name });
    for (body, fields) in [
        (
            create("", ""),
            json!({ "slug": "required", "name": "required" }),
        ),
        (create("ab", "x"), json!({ "slug": "invalid" })),
        (create("Firma", "x"), json!({ "slug": "invalid" })),
        (create("-firma", "x"), json!({ "slug": "invalid" })),
        (create("admin", "x"), json!({ "slug": "reserved" })),
        (create("acme", "x"), json!({ "slug": "taken" })),
        (
            create("firma", &"n".repeat(201)),
            json!({ "name": "too_long" }),
        ),
    ] {
        let (s, j) = on(
            &app,
            Method::POST,
            BASE_HOST,
            "/api/spaces",
            As::Cookie(&cookie),
            Some(body.clone()),
        )
        .await;
        assert_eq!(
            (s, &j["fields"]),
            (StatusCode::UNPROCESSABLE_ENTITY, &fields),
            "{body}"
        );
    }
    let (s, j) = on(
        &app,
        Method::POST,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&cookie),
        Some(create(" firma-2 ", " Firma 2 ")),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    assert_eq!(j["slug"], "firma-2");
    let (_, list) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(&cookie),
        None,
    )
    .await;
    let slugs: Vec<&str> = list
        .as_array()
        .expect("list")
        .iter()
        .filter_map(|s| s["slug"].as_str())
        .collect();
    assert_eq!(
        slugs,
        ["firma-2", "acme"],
        "by name: \"Firma 2\" < \"Space acme\""
    );
    let (s, _) = on(
        &app,
        Method::GET,
        BASE_HOST,
        "/api/spaces",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn current_space_get_and_rename() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (s, j) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/space",
        As::Bearer(TEST_TOKEN),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        j,
        json!({ "slug": "acme", "name": "Space acme", "role": "owner", "url": "http://acme.localhost:3000", "requireMfa": false })
    );
    let (s, j) = on(
        &app,
        Method::PUT,
        TEST_HOST,
        "/api/space",
        As::Bearer(TEST_TOKEN),
        Some(json!({ "name": " Acme s.r.o. " })),
    )
    .await;
    assert_eq!((s, &j["name"]), (StatusCode::OK, &json!("Acme s.r.o.")));
    let (s, j) = on(
        &app,
        Method::PUT,
        TEST_HOST,
        "/api/space",
        As::Bearer(TEST_TOKEN),
        Some(json!({ "name": "" })),
    )
    .await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "name": "required" })
        )
    );
    let member = member(&db.conn, db.space, "m@example.com", Role::Member).await;
    let (s, j) = on(
        &app,
        Method::PUT,
        TEST_HOST,
        "/api/space",
        As::Bearer(&member),
        Some(json!({ "name": "x" })),
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }))
    );
    let (s, j) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/space",
        As::Bearer(&member),
        None,
    )
    .await;
    assert_eq!((s, &j["role"]), (StatusCode::OK, &json!("member")));
}
