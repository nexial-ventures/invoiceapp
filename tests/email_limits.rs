//! Outbound e-mail limits (spaces.md "Limits"): per space and per sending
//! user within the space, over a rolling hour and day, counted in `email_sends`; document
//! e-mails, the settings test e-mail and invitations all count.

mod common;

use common::auth::{As, member, seed_space, user};
use common::documents::{create_issued, id};
use common::email_limits::{age_sends, assert_limited, post, post_on, seed_sends, sends};
use common::members::{B_HOST, count, join, token};
use common::smtp::{EmailEnv, issuable_with_emails, send_body};
use common::{TEST_TOKEN, TestDb};
use invoice::space::Role;
use serde_json::json;

use axum::http::StatusCode;
use sea_orm::ConnectionTrait;

const OWNER: As<'static> = As::Bearer(TEST_TOKEN);

/// The app with SMTP and an issued document to send; its send URI.
async fn setup(db: &TestDb, env: &EmailEnv) -> (axum::Router, String) {
    let app = env.router(db.conn.clone());
    let doc = create_issued(&app, issuable_with_emails(&app).await).await;
    (app, format!("/api/documents/{}/email", id(&doc)))
}

async fn sql(db: &TestDb, stmt: &str) {
    db.conn.execute_unprepared(stmt).await.expect(stmt);
}

#[tokio::test]
async fn space_hour_window() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let (app, uri) = setup(&db, &env).await;
    // Another member of the space sent 19 ten minutes ago.
    let other = user(&db.conn, "other@example.com").await;
    seed_sends(&db.conn, Some(db.space.uuid()), other, 19, "10 minutes").await;

    let (s, _, j) = post(&app, OWNER, &uri, send_body(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "the 20th: {j}");
    assert_limited(post(&app, OWNER, &uri, send_body(json!({}))).await, 50 * 60);
    assert_eq!(env.smtp.mails().len(), 1, "the refused one is not sent");
    assert_eq!(sends(&db.conn, None).await, 20, "nor counted");

    // The operator raises the space's hourly limit.
    sql(&db, "UPDATE space_limits SET email_space_per_hour = 21").await;
    assert_eq!(
        post(&app, OWNER, &uri, send_body(json!({}))).await.0,
        StatusCode::OK
    );
    assert_eq!(
        post(&app, OWNER, &uri, send_body(json!({}))).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );

    // An hour later the window is free again (the day still has room).
    age_sends(&db.conn, "1 hour").await;
    assert_eq!(
        post(&app, OWNER, &uri, send_body(json!({}))).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn space_day_window() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let (app, uri) = setup(&db, &env).await;
    let other = user(&db.conn, "other@example.com").await;
    seed_sends(&db.conn, Some(db.space.uuid()), other, 99, "2 hours").await;

    assert_eq!(
        post(&app, OWNER, &uri, send_body(json!({}))).await.0,
        StatusCode::OK
    );
    // The oldest leaves the day window in 22 h.
    assert_limited(
        post(&app, OWNER, &uri, send_body(json!({}))).await,
        22 * 3600,
    );
    age_sends(&db.conn, "22 hours").await;
    assert_eq!(
        post(&app, OWNER, &uri, send_body(json!({}))).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn user_limits_count_per_space() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let (app, uri) = setup(&db, &env).await;
    // Only the user limits can bite here.
    sql(
        &db,
        "UPDATE space_limits SET email_space_per_hour = 1000, email_space_per_day = 1000",
    )
    .await;
    // The owner sent 20 from this space half an hour ago (and 50 from a
    // space deleted since, which count nowhere).
    seed_sends(
        &db.conn,
        Some(db.space.uuid()),
        db.user_id,
        20,
        "30 minutes",
    )
    .await;
    seed_sends(&db.conn, None, db.user_id, 50, "10 minutes").await;
    assert_limited(post(&app, OWNER, &uri, send_body(json!({}))).await, 30 * 60);
    // Another member of the same space is not limited by the owner's count.
    let jana = member(&db.conn, db.space, "jana@example.com", Role::Member).await;
    let (s, _, j) = post(&app, As::Bearer(&jana), &uri, send_body(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{j}");

    // The owner's budget in another space is separate.
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", None).await;
    join(&db.conn, beta, db.user_id, Role::Owner).await;
    let in_beta = token(&db.conn, beta, db.user_id, Role::Owner).await;
    let test = json!({ "to": "me@example.com" });
    let (s, _, j) = post_on(
        &app,
        B_HOST,
        As::Bearer(&in_beta),
        "/api/settings/email/test",
        test,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");

    // The day: 100 of the owner's in this space within 24 h, none in the last hour.
    age_sends(&db.conn, "2 hours").await;
    seed_sends(&db.conn, Some(db.space.uuid()), db.user_id, 80, "3 hours").await;
    // The oldest of them leaves the day window in 21 h.
    assert_limited(
        post(&app, OWNER, &uri, send_body(json!({}))).await,
        21 * 3600,
    );
    sql(&db, "UPDATE space_limits SET email_user_per_day = 200").await;
    assert_eq!(
        post(&app, OWNER, &uri, send_body(json!({}))).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn refused_invitations_give_the_reservation_back() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());
    let unknown = format!("/api/invites/{}/resend", uuid::Uuid::new_v4());
    let (s, _, _) = post(&app, OWNER, &unknown, json!({})).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(sends(&db.conn, None).await, 0);

    // An admin may neither replace nor resend the owner's owner invitation.
    let owner_invite = json!({ "email": "x@example.com", "role": "owner" });
    let (s, _, j) = post(&app, OWNER, "/api/invites", owner_invite).await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    assert_eq!(sends(&db.conn, None).await, 1);
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let replace = json!({ "email": "x@example.com", "role": "member" });
    let (s, _, _) = post(&app, As::Bearer(&admin), "/api/invites", replace).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let resend = format!("/api/invites/{}/resend", j["id"].as_str().expect("id"));
    let (s, _, _) = post(&app, As::Bearer(&admin), &resend, json!({})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(
        sends(&db.conn, None).await,
        1,
        "the refused ones are not counted"
    );
}

#[tokio::test]
async fn test_and_invitation_emails_count() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let (app, _) = setup(&db, &env).await;
    let other = user(&db.conn, "other@example.com").await;
    seed_sends(&db.conn, Some(db.space.uuid()), other, 19, "5 minutes").await;

    let test = "/api/settings/email/test";
    let (s, _, j) = post(&app, OWNER, test, json!({ "to": "me@example.com" })).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{j}");
    assert_eq!(sends(&db.conn, Some("test")).await, 1);
    assert_limited(post(&app, OWNER, test, json!({})).await, 55 * 60);

    let invite = json!({ "email": "new@example.com", "role": "member" });
    assert_limited(
        post(&app, OWNER, "/api/invites", invite.clone()).await,
        55 * 60,
    );
    let pending = "SELECT count(*) FROM space_invites";
    assert_eq!(count(&db.conn, pending).await, 0, "nothing created");

    age_sends(&db.conn, "1 hour").await;
    let (s, _, j) = post(&app, OWNER, "/api/invites", invite.clone()).await;
    assert_eq!(
        (s, &j["emailSent"]),
        (StatusCode::CREATED, &json!(true)),
        "{j}"
    );
    assert_eq!(sends(&db.conn, Some("invite")).await, 1);

    // Without SMTP nothing is mailed, so nothing is counted.
    let off = env.unconfigured(db.conn.clone());
    let (s, _, j) = post(&off, OWNER, "/api/invites", invite).await;
    assert_eq!(
        (s, &j["emailSent"]),
        (StatusCode::CREATED, &json!(false)),
        "{j}"
    );
    assert_eq!(sends(&db.conn, Some("invite")).await, 1);
}

#[tokio::test]
async fn ten_recipients_at_most() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let (app, uri) = setup(&db, &env).await;
    let addr = |n: usize| {
        (0..n)
            .map(|i| format!("a{i}@example.com"))
            .collect::<Vec<_>>()
    };
    let over = json!({ "to": addr(4), "cc": addr(4), "bcc": addr(3) });
    let (s, _, j) = post(&app, OWNER, &uri, send_body(over)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{j}");
    assert_eq!(j["fields"]["to"], "too_long");
    assert_eq!(
        sends(&db.conn, None).await,
        0,
        "a refused body is not counted"
    );

    let ten = json!({ "to": addr(4), "cc": addr(3), "bcc": addr(3) });
    let (s, _, j) = post(&app, OWNER, &uri, send_body(ten)).await;
    assert_eq!(s, StatusCode::OK, "{j}");
    assert_eq!(
        sends(&db.conn, Some("document")).await,
        1,
        "one e-mail, however many recipients"
    );
}
