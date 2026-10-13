//! Personal API token routes and expiry (spaces.md, 4a); a disabled user's
//! credentials stop working.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, member, on, status};
use common::members::{session_on, user_id};
use common::{BASE_HOST, TEST_HOST, TEST_TOKEN, TestDb, router};
use invoiceapp::auth::session;
use invoiceapp::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

#[tokio::test]
async fn token_routes() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mem = member(&db.conn, db.space, "mem@example.com", Role::Member).await;
    let create = |name: &str, role: &str, exp: Value| json!({ "name": name, "role": role, "expiresAt": exp });

    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(&mem),
        Some(create("x", "admin", Value::Null)),
    )
    .await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "role": "too_high" })
        )
    );
    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(&mem),
        Some(create("", "boss", json!("2000-01-01"))),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        j["fields"],
        json!({ "name": "required", "role": "invalid", "expiresAt": "invalid" })
    );

    let today = chrono::Utc::now().date_naive().to_string();
    let (s, created) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(&mem),
        Some(create("MCP", "member", json!(today))),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let token = created["token"]
        .as_str()
        .expect("token shown once")
        .to_string();
    assert!(token.starts_with(&format!(
        "inv_{}_",
        created["prefix"].as_str().expect("prefix")
    )));
    assert_eq!(created["expiresAt"], json!(today));
    assert_eq!(
        (created["role"].clone(), created["lastUsedAt"].clone()),
        (json!("member"), Value::Null)
    );
    assert_eq!(
        status(&app, Method::GET, "/api/contacts", &token, None).await,
        StatusCode::OK,
        "valid through its last day"
    );

    // Own tokens for a member; every token with its user for admin+.
    let (_, own) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(&mem),
        None,
    )
    .await;
    let own = own.as_array().expect("list").clone();
    assert_eq!(own.len(), 2, "the member's seeded token and the new one");
    assert!(
        own.iter()
            .all(|t| t.get("user").is_none() && t.get("token").is_none())
    );
    assert!(
        own.iter().any(|t| t["lastUsedAt"].is_string()),
        "used tokens show lastUsedAt"
    );
    let (_, all) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/tokens",
        As::Bearer(TEST_TOKEN),
        None,
    )
    .await;
    let all = all.as_array().expect("list").clone();
    assert_eq!(all.len(), 3);
    assert!(all.iter().any(|t| t["user"] == json!({ "email": "mem@example.com", "displayName": "Test User" })));

    // A member cannot revoke someone else's token (404); admin+ can.
    let owner_token_id = all
        .iter()
        .find(|t| t["user"]["email"] == "owner@example.com")
        .expect("owner's")["id"]
        .clone();
    let uri = |id: &Value| format!("/api/tokens/{}", id.as_str().expect("id"));
    assert_eq!(
        status(&app, Method::DELETE, &uri(&owner_token_id), &mem, None).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, Method::DELETE, &uri(&created["id"]), &mem, None).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(&app, Method::GET, "/api/contacts", &token, None).await,
        StatusCode::UNAUTHORIZED,
        "revoked"
    );
    let mem_id = own
        .iter()
        .find(|t| t["id"] != created["id"])
        .expect("seeded")["id"]
        .clone();
    assert_eq!(
        status(&app, Method::DELETE, &uri(&mem_id), TEST_TOKEN, None).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(&app, Method::GET, "/api/contacts", &mem, None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(
            &app,
            Method::DELETE,
            "/api/tokens/not-a-uuid",
            TEST_TOKEN,
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn expired_tokens_are_refused() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    db.conn
        .execute_unprepared("UPDATE api_tokens SET expires_at = current_date - 1")
        .await
        .expect("expire");
    let (s, j) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/contacts",
        As::Bearer(TEST_TOKEN),
        None,
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::UNAUTHORIZED, json!({ "code": "unauthorized" }))
    );
}

#[tokio::test]
async fn disabled_user_loses_session_and_tokens() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let token = member(&db.conn, db.space, "mem@example.com", Role::Member).await;
    let user = user_id(&db.conn, "mem@example.com").await;
    let cookie = session_on(&db.conn, db.space, user).await;
    // A base-host session (no space) is resolved by a separate branch.
    let base_cookie = session::create(&db.conn, user, None, None, session::Method::Password)
        .await
        .expect("base-host session");
    let calls = [
        (TEST_HOST, "/api/contacts", As::Bearer(&token)),
        (TEST_HOST, "/api/contacts", As::Cookie(&cookie)),
        (BASE_HOST, "/api/auth/me", As::Cookie(&base_cookie)),
    ];
    for (host, uri, who) in calls {
        let (s, j) = on(&app, Method::GET, host, uri, who, None).await;
        assert_eq!(s, StatusCode::OK, "{host}{uri} works before: {j}");
    }

    db.conn
        .execute_unprepared(&format!(
            "UPDATE users SET disabled = true WHERE id = '{user}'"
        ))
        .await
        .expect("disable");
    for (host, uri, who) in calls {
        let (s, j) = on(&app, Method::GET, host, uri, who, None).await;
        assert_eq!(
            (s, j),
            (StatusCode::UNAUTHORIZED, json!({ "code": "unauthorized" })),
            "{host}{uri}"
        );
    }
    assert_eq!(
        status(&app, Method::GET, "/api/contacts", TEST_TOKEN, None).await,
        StatusCode::OK,
        "other users unaffected"
    );
}
