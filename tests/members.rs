//! Members: list, role rules, last-owner guard, removal and leaving
//! (members.md, 4b).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, member, on, seed_space};
use common::members::{session_on, token, user_id};
use common::{OWNER_EMAIL, TEST_HOST, TEST_TOKEN, TestDb, router};
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
async fn list_and_role_rules() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let mem = member(&db.conn, db.space, "mem@example.com", Role::Member).await;
    member(&db.conn, db.space, "acc@example.com", Role::Accountant).await;
    let mem_id = user_id(&db.conn, "mem@example.com").await;
    let acc_id = user_id(&db.conn, "acc@example.com").await;
    let owner = As::Bearer(TEST_TOKEN);

    let (s, list) = on(&app, Method::GET, TEST_HOST, "/api/members", owner, None).await;
    assert_eq!(s, StatusCode::OK);
    let rows: Vec<(String, String, bool)> = list
        .as_array()
        .expect("array")
        .iter()
        .map(|m| {
            (
                m["email"].as_str().unwrap_or_default().to_string(),
                m["role"].as_str().unwrap_or_default().to_string(),
                m["isSelf"].as_bool().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (OWNER_EMAIL.into(), "owner".into(), true),
            ("admin@example.com".into(), "admin".into(), false),
            ("mem@example.com".into(), "member".into(), false),
            ("acc@example.com".into(), "accountant".into(), false),
        ]
    );
    assert_eq!(list[0]["displayName"], "Test User");
    assert!(list[0]["joinedAt"].is_string() && list[0]["userId"] == json!(db.user_id));

    // Below admin: no member management at all.
    let (s, j) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/members",
        As::Bearer(&mem),
        None,
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }))
    );
    assert_eq!(
        put_role(&app, As::Bearer(&mem), acc_id, "member").await.0,
        StatusCode::FORBIDDEN
    );

    // Admin: up to admin, never an owner.
    let (s, j) = put_role(&app, As::Bearer(&admin), mem_id, "admin").await;
    assert_eq!(s, StatusCode::OK, "{j}");
    assert_eq!(
        (j["role"].clone(), j["isSelf"].clone()),
        (json!("admin"), json!(false))
    );
    let (s, j) = put_role(&app, As::Bearer(&admin), acc_id, "owner").await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "role": "too_high" })
        )
    );
    let (s, j) = put_role(&app, As::Bearer(&admin), db.user_id, "member").await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }))
    );
    assert_eq!(
        remove(&app, As::Bearer(&admin), db.user_id).await.0,
        StatusCode::FORBIDDEN
    );
    let (s, j) = put_role(&app, As::Bearer(&admin), acc_id, "boss").await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "role": "invalid" })
        )
    );
    let (s, j) = on(
        &app,
        Method::PUT,
        TEST_HOST,
        &format!("/api/members/{acc_id}"),
        As::Bearer(&admin),
        Some(json!({})),
    )
    .await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "role": "required" })
        )
    );
    assert_eq!(
        put_role(&app, As::Bearer(&admin), uuid::Uuid::new_v4(), "member")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (s, _) = on(
        &app,
        Method::PUT,
        TEST_HOST,
        "/api/members/nope",
        As::Bearer(&admin),
        Some(json!({ "role": "member" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // The owner grants owner; an admin may change their own role downwards.
    let (s, j) = put_role(&app, owner, mem_id, "owner").await;
    assert_eq!((s, j["role"].clone()), (StatusCode::OK, json!("owner")));
    let admin_id = user_id(&db.conn, "admin@example.com").await;
    let (s, j) = put_role(&app, As::Bearer(&admin), admin_id, "member").await;
    assert_eq!((s, j["isSelf"].clone()), (StatusCode::OK, json!(true)));

    // Removing oneself is leaving, not removal.
    let (s, j) = remove(&app, owner, db.user_id).await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }))
    );
    assert_eq!(remove(&app, owner, acc_id).await.0, StatusCode::NO_CONTENT);
    assert_eq!(remove(&app, owner, acc_id).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_last_owner_stays() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let owner = As::Bearer(TEST_TOKEN);

    let (s, j) = put_role(&app, owner, db.user_id, "admin").await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "role": "last_owner" })
        )
    );
    let cookie = session_on(&db.conn, db.space, db.user_id).await;
    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/space/leave",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::CONFLICT, json!({ "code": "last_owner" }))
    );

    // With a second owner: one may step down or leave, the other stays.
    member(&db.conn, db.space, "co@example.com", Role::Owner).await;
    let co = user_id(&db.conn, "co@example.com").await;
    let (s, j) = put_role(&app, owner, db.user_id, "admin").await;
    assert_eq!(
        (s, j["role"].clone()),
        (StatusCode::OK, json!("admin")),
        "{j}"
    );
    // Now an admin: cannot touch the remaining owner.
    assert_eq!(remove(&app, owner, co).await.0, StatusCode::FORBIDDEN);
    let co_token = token(&db.conn, db.space, co, Role::Owner).await;
    let (s, j) = put_role(&app, As::Bearer(&co_token), co, "member").await;
    assert_eq!(j["fields"], json!({ "role": "last_owner" }), "{s}");
    let co_cookie = session_on(&db.conn, db.space, co).await;
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/space/leave",
        As::Cookie(&co_cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Two owners demoting each other at once: one of them stays owner.
    let (s, _) = put_role(&app, As::Bearer(&co_token), db.user_id, "owner").await;
    assert_eq!(s, StatusCode::OK);
    let (a, b) = tokio::join!(
        put_role(&app, owner, co, "admin"),
        put_role(&app, As::Bearer(&co_token), db.user_id, "admin"),
    );
    let ok = [a.0, b.0].iter().filter(|s| **s == StatusCode::OK).count();
    assert_eq!(ok, 1, "{a:?} {b:?}");
    let owners = common::members::count(
        &db.conn,
        &format!(
            "SELECT count(*) FROM space_members WHERE space_id = '{}' AND role = 'owner'",
            db.space
        ),
    )
    .await;
    assert_eq!(owners, 1);
}

#[tokio::test]
async fn another_space_members_are_not_found() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (_, beta_owner) = seed_space(&db.conn, "beta", "beta@example.com", None).await;
    let owner = As::Bearer(TEST_TOKEN);
    assert_eq!(
        put_role(&app, owner, beta_owner, "member").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        remove(&app, owner, beta_owner).await.0,
        StatusCode::NOT_FOUND
    );
    let (s, list) = on(&app, Method::GET, TEST_HOST, "/api/members", owner, None).await;
    assert_eq!(
        (s, list.as_array().map(Vec::len)),
        (StatusCode::OK, Some(1))
    );
    // Base host: the route does not exist.
    let (s, _) = on(
        &app,
        Method::GET,
        common::BASE_HOST,
        "/api/members",
        owner,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_disabled_owner_does_not_count() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    member(&db.conn, db.space, "gone@example.com", Role::Owner).await;
    sea_orm::ConnectionTrait::execute_unprepared(
        &db.conn,
        "UPDATE users SET disabled = true WHERE email = 'gone@example.com'",
    )
    .await
    .expect("disable");
    let (s, j) = put_role(&app, As::Bearer(TEST_TOKEN), db.user_id, "admin").await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "role": "last_owner" })
        )
    );
    let cookie = session_on(&db.conn, db.space, db.user_id).await;
    let (s, _) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/space/leave",
        As::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}
