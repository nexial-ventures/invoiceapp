//! A kept invitation link never outlives its inviter's right to grant the
//! role, and invitation e-mails are rate limited (members.md, 4b).

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, member, on};
use common::members::{accept, count, invite, link_token, show, user_id};
use common::{TEST_HOST, TEST_TOKEN, TestDb, router};
use invoiceapp::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::json;

const PASSWORD: &str = "a brand new password";

fn body(token: &str) -> serde_json::Value {
    json!({ "token": token, "password": PASSWORD, "displayName": "Eve" })
}

async fn invited_by(app: &axum::Router, who: &str, email: &str, role: &str) -> String {
    let (s, j) = invite(app, As::Bearer(who), email, role).await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    link_token(&j["url"])
}

#[tokio::test]
async fn removal_and_demotion_delete_the_inviters_invitations() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let admin_id = user_id(&db.conn, "admin@example.com").await;
    let as_admin = invited_by(&app, &admin, "mem@example.com", "member").await;
    let as_admin2 = invited_by(&app, &admin, "adm@example.com", "admin").await;
    let by_owner = invited_by(&app, TEST_TOKEN, "own@example.com", "admin").await;

    // Demoted to admin → member: may grant nothing, every invitation dies.
    let uri = format!("/api/members/{admin_id}");
    let demote = Some(json!({ "role": "member" }));
    let owner = As::Bearer(TEST_TOKEN);
    assert_eq!(
        on(&app, Method::PUT, TEST_HOST, &uri, owner, demote)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(show(&app, &as_admin).await.0, StatusCode::NOT_FOUND);
    assert_eq!(show(&app, &as_admin2).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        show(&app, &by_owner).await.0,
        StatusCode::OK,
        "others' stay"
    );

    // Removed: the invitations they sent go with them.
    let promote = Some(json!({ "role": "admin" }));
    assert_eq!(
        on(&app, Method::PUT, TEST_HOST, &uri, owner, promote)
            .await
            .0,
        StatusCode::OK
    );
    let again = invited_by(&app, &admin, "mem@example.com", "member").await;
    assert_eq!(
        on(&app, Method::DELETE, TEST_HOST, &uri, owner, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(show(&app, &again).await.0, StatusCode::NOT_FOUND);
    let (s, _, j, _) = accept(&app, TEST_HOST, body(&again)).await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "token": "invalid" })
        )
    );
}

#[tokio::test]
async fn accept_rechecks_the_inviter() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let admin_id = user_id(&db.conn, "admin@example.com").await;
    let token = invited_by(&app, &admin, "eve@example.com", "admin").await;
    // The inviter loses the right to grant `admin` behind the API's back.
    db.conn
        .execute_unprepared(&format!(
            "UPDATE space_members SET role = 'member' WHERE user_id = '{admin_id}'"
        ))
        .await
        .expect("demote");
    assert_eq!(show(&app, &token).await.0, StatusCode::OK);
    let (s, cookie, j, _) = accept(&app, TEST_HOST, body(&token)).await;
    assert_eq!((s, cookie), (StatusCode::UNPROCESSABLE_ENTITY, None));
    assert_eq!(j["fields"], json!({ "token": "invalid" }));
    assert_eq!(
        count(&db.conn, "SELECT count(*) FROM space_invites").await,
        0,
        "deleted"
    );
    let eve = "SELECT count(*) FROM users WHERE email = 'eve@example.com'";
    assert_eq!(count(&db.conn, eve).await, 0, "no account created");

    // A disabled inviter grants nothing either.
    let token = invited_by(&app, TEST_TOKEN, "eve@example.com", "member").await;
    db.conn
        .execute_unprepared("UPDATE users SET disabled = true WHERE email = 'owner@example.com'")
        .await
        .expect("disable");
    assert_eq!(
        accept(&app, TEST_HOST, body(&token)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn invitation_emails_are_limited_per_user_and_per_space() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let a = member(&db.conn, db.space, "a@example.com", Role::Admin).await;
    let b = member(&db.conn, db.space, "b@example.com", Role::Admin).await;
    let owner = As::Bearer(TEST_TOKEN);
    for i in 0..20 {
        let (s, _) = invite(&app, owner, &format!("o{i}@example.com"), "member").await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let (s, j) = invite(&app, owner, "o0@example.com", "member").await;
    assert_eq!(
        (s, &j["code"]),
        (StatusCode::TOO_MANY_REQUESTS, &json!("rate_limited"))
    );
    // A resend counts too (in a's bucket).
    let (_, list) = on(&app, Method::GET, TEST_HOST, "/api/invites", owner, None).await;
    let id = list[0]["id"].as_str().expect("id");
    let uri = format!("/api/invites/{id}/resend");
    let (s, j) = on(&app, Method::POST, TEST_HOST, &uri, As::Bearer(&a), None).await;
    assert_eq!(s, StatusCode::OK, "another user's bucket: {j}");
    for i in 0..19 {
        let (s, _) = invite(&app, As::Bearer(&a), &format!("a{i}@example.com"), "member").await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let (s, j) = invite(&app, As::Bearer(&a), "ax@example.com", "member").await;
    assert_eq!(
        (s, &j["code"]),
        (StatusCode::TOO_MANY_REQUESTS, &json!("rate_limited"))
    );
    // 40 in the space so far (refused requests do not count): b gets 10.
    for i in 0..10 {
        let (s, _) = invite(&app, As::Bearer(&b), &format!("b{i}@example.com"), "member").await;
        assert_eq!(s, StatusCode::CREATED, "{i}");
    }
    let resp = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/invites",
        As::Bearer(&b),
        Some(json!({ "email": "bx@example.com", "role": "member" })),
    )
    .await;
    assert_eq!(
        resp,
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "code": "rate_limited" })
        )
    );
}
