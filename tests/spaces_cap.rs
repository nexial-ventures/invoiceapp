//! The space cap (spaces.md "Limits"): a user may own at most
//! `INVOICE__MAX_SPACES_PER_USER` (default 3) spaces they created;
//! memberships via invitations — even as owner — do not count.

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use common::auth::{As, login, on, seed_space};
use common::members::{count, join};
use common::{BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD, TestDb};
use invoice::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

async fn create(app: &Router, cookie: &str, slug: &str) -> (StatusCode, Value) {
    on(
        app,
        Method::POST,
        BASE_HOST,
        "/api/spaces",
        As::Cookie(cookie),
        Some(json!({ "slug": slug, "name": slug })),
    )
    .await
}

#[tokio::test]
async fn owning_counts_invitations_do_not() {
    let db = TestDb::new().await;
    let app = common::router_open(db.conn.clone());
    let cookie = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    // The owner created `acme`; they are also an owner of `beta`, which
    // someone else created and invited them into.
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", None).await;
    join(&db.conn, beta, db.user_id, Role::Owner).await;

    for slug in ["druha", "treti"] {
        let (s, j) = create(&app, &cookie, slug).await;
        assert_eq!(s, StatusCode::CREATED, "{slug}: {j}");
    }
    let (s, j) = create(&app, &cookie, "ctvrta").await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "space_limit" }))
    );
    // Validation runs first: a reserved slug is still a 422.
    let (s, j) = create(&app, &cookie, "paypal").await;
    assert_eq!(
        (s, &j["fields"]["slug"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("reserved"))
    );

    // Handing a created space over (no longer its owner) frees a slot…
    db.conn
        .execute_unprepared(&format!(
            "UPDATE space_members SET role = 'admin' WHERE user_id = '{}' \
             AND space_id = (SELECT id FROM spaces WHERE slug = 'druha')",
            db.user_id
        ))
        .await
        .expect("demote");
    assert_eq!(create(&app, &cookie, "ctvrta").await.0, StatusCode::CREATED);
    assert_eq!(create(&app, &cookie, "pata").await.0, StatusCode::FORBIDDEN);
    // …and so does deleting one.
    db.conn
        .execute_unprepared("DELETE FROM spaces WHERE slug = 'treti'")
        .await
        .expect("delete");
    assert_eq!(create(&app, &cookie, "pata").await.0, StatusCode::CREATED);
}

#[tokio::test]
async fn parallel_creations_cannot_pass_the_cap() {
    let db = TestDb::new().await;
    let app = common::router_open(db.conn.clone());
    let cookie = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    // `acme` is one of three: two slots left for five parallel requests.
    let calls = (0..5).map(|n| {
        let (app, cookie) = (app.clone(), cookie.clone());
        async move { create(&app, &cookie, &format!("par-{n}")).await.0 }
    });
    let statuses = futures_util::future::join_all(calls).await;
    let created = statuses
        .iter()
        .filter(|s| **s == StatusCode::CREATED)
        .count();
    let refused = statuses
        .iter()
        .filter(|s| **s == StatusCode::FORBIDDEN)
        .count();
    assert_eq!((created, refused), (2, 3), "{statuses:?}");
    let owned = format!(
        "SELECT count(*) FROM spaces WHERE created_by = '{}'",
        db.user_id
    );
    assert_eq!(count(&db.conn, &owned).await, 3);
}

#[tokio::test]
async fn a_space_with_a_newly_reserved_slug_still_resolves() {
    let db = TestDb::new().await;
    let app = common::router_open(db.conn.clone());
    // Created before `demo` was reserved.
    seed_space(&db.conn, "demo", "demo@example.com", None).await;
    let (s, j) = on(
        &app,
        Method::GET,
        "demo.localhost:3000",
        "/api/context",
        As::Nobody,
        None,
    )
    .await;
    assert_eq!(
        (s, &j["space"]["slug"]),
        (StatusCode::OK, &json!("demo")),
        "{j}"
    );
    // A new space cannot take such a name.
    let cookie = login(&app, BASE_HOST, OWNER_EMAIL, OWNER_PASSWORD)
        .await
        .expect("login");
    let (s, j) = create(&app, &cookie, "test").await;
    assert_eq!(
        (s, &j["fields"]["slug"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("reserved"))
    );
}
