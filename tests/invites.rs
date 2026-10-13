//! Invitations: create, replace, list, resend, revoke, validation, e-mail,
//! isolation (members.md, 4b). Accepting: `invites_accept.rs`.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, mail, mail_app, mail_text, mail_token, member, on, seed_space};
use common::members::{B_HOST, count, invite, link_token, show};
use common::smtp::EmailEnv;
use common::{OWNER_EMAIL, TEST_HOST, TEST_TOKEN, TestDb, router};
use invoiceapp::space::Role;
use serde_json::json;

const OWNER: As<'static> = As::Bearer(TEST_TOKEN);

#[tokio::test]
async fn invitation_is_mailed_and_listed() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = mail_app(&db.conn, &env, false);

    let body = json!({ "email": " Nova@Example.com ", "role": "member", "locale": "en" });
    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/invites",
        OWNER,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    assert_eq!(j["email"], "nova@example.com");
    assert_eq!(j["role"], "member");
    assert_eq!(j["emailSent"], true);
    assert!(
        j.get("url").is_none(),
        "with SMTP the link goes only by e-mail"
    );
    assert_eq!(
        j["invitedBy"],
        json!({ "email": OWNER_EMAIL, "displayName": "Test User" })
    );
    let created = chrono::DateTime::parse_from_rfc3339(j["createdAt"].as_str().expect("createdAt"))
        .expect("rfc3339");
    let expires = chrono::DateTime::parse_from_rfc3339(j["expiresAt"].as_str().expect("expiresAt"))
        .expect("rfc3339");
    assert_eq!(expires - created, chrono::Duration::days(7));

    let m = mail(&env.smtp, 1).await;
    assert_eq!(m.rcpt, ["<nova@example.com>"]);
    let text = mail_text(&m);
    let first = mail_token(&m, "invite");
    assert!(
        text.contains(&format!("http://acme.localhost:3000/invite?token={first}"))
            && text.contains("Space acme")
            && text.contains("Test User"),
        "{text}"
    );
    assert!(text.contains("member"), "{text}");
    let subject = m.header("subject").unwrap_or_default();
    assert!(subject.contains("invites you to"), "{subject}");

    let (s, list) = on(&app, Method::GET, TEST_HOST, "/api/invites", OWNER, None).await;
    assert_eq!(s, StatusCode::OK);
    let items = list.as_array().expect("array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], j["id"]);
    assert!(items[0].get("url").is_none(), "the link is shown only once");

    // Resend: a new token, mailed again (cs by default), the old link dies.
    let id = j["id"].as_str().expect("id");
    let uri = format!("/api/invites/{id}/resend");
    let (s, r) = on(&app, Method::POST, TEST_HOST, &uri, OWNER, None).await;
    assert_eq!(
        (s, r["emailSent"].clone(), r.get("url")),
        (StatusCode::OK, json!(true), None),
        "{r}"
    );
    let m = mail(&env.smtp, 2).await;
    assert!(mail_text(&m).contains("vás zve"), "{}", mail_text(&m));
    let second = mail_token(&m, "invite");
    assert_ne!(second, first);
    assert_eq!(show(&app, &first).await.0, StatusCode::NOT_FOUND);
    assert_eq!(show(&app, &second).await.0, StatusCode::OK);
}

#[tokio::test]
async fn without_smtp_the_invitation_is_created_but_not_mailed() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (s, j) = invite(&app, OWNER, "nova@example.com", "accountant").await;
    assert_eq!(
        (s, j["emailSent"].clone()),
        (StatusCode::CREATED, json!(false))
    );
    let url = j["url"]
        .as_str()
        .expect("without SMTP the link is returned");
    assert!(
        url.starts_with("http://acme.localhost:3000/invite?token="),
        "{url}"
    );
    assert_eq!(show(&app, &link_token(&j["url"])).await.0, StatusCode::OK);
}

#[tokio::test]
async fn a_new_invitation_replaces_the_pending_one() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (_, first) = invite(&app, OWNER, "nova@example.com", "member").await;
    let (s, second) = invite(&app, OWNER, "NOVA@example.com", "admin").await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(second["id"], first["id"], "replaced in place");
    assert_eq!(
        show(&app, &link_token(&first["url"])).await.0,
        StatusCode::NOT_FOUND
    );
    let (s, info) = show(&app, &link_token(&second["url"])).await;
    assert_eq!((s, info["role"].clone()), (StatusCode::OK, json!("admin")));
    let (_, list) = on(&app, Method::GET, TEST_HOST, "/api/invites", OWNER, None).await;
    assert_eq!(list.as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn expired_invitations_are_gone_and_revoked_ones_too() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (_, a) = invite(&app, OWNER, "a@example.com", "member").await;
    let (_, b) = invite(&app, OWNER, "b@example.com", "member").await;
    let a_id = a["id"].as_str().expect("id");
    sea_orm::ConnectionTrait::execute_unprepared(
        &db.conn,
        &format!(
            "UPDATE space_invites SET expires_at = now() - interval '1 second' WHERE id = '{a_id}'"
        ),
    )
    .await
    .expect("expire");
    assert_eq!(
        show(&app, &link_token(&a["url"])).await.0,
        StatusCode::NOT_FOUND
    );
    let uri = format!("/api/invites/{a_id}/resend");
    assert_eq!(
        on(&app, Method::POST, TEST_HOST, &uri, OWNER, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, list) = on(&app, Method::GET, TEST_HOST, "/api/invites", OWNER, None).await;
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    assert_eq!(list[0]["id"], b["id"]);
    assert_eq!(
        count(&db.conn, "SELECT count(*) FROM space_invites").await,
        1,
        "purged"
    );

    let uri = format!("/api/invites/{}", b["id"].as_str().expect("id"));
    assert_eq!(
        on(&app, Method::DELETE, TEST_HOST, &uri, OWNER, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        show(&app, &link_token(&b["url"])).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        on(&app, Method::DELETE, TEST_HOST, &uri, OWNER, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn validation_and_role_rules() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let mem = member(&db.conn, db.space, "mem@example.com", Role::Member).await;

    let (s, j) = on(
        &app,
        Method::POST,
        TEST_HOST,
        "/api/invites",
        OWNER,
        Some(json!({})),
    )
    .await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "email": "required", "role": "required" })
        )
    );
    let (_, j) = invite(&app, OWNER, "nope", "boss").await;
    assert_eq!(
        j["fields"],
        json!({ "email": "invalid", "role": "invalid" })
    );
    let (_, j) = invite(&app, OWNER, " Mem@Example.com", "member").await;
    assert_eq!(j["fields"], json!({ "email": "already_member" }));
    let (s, j) = invite(&app, As::Bearer(&admin), "new@example.com", "owner").await;
    assert_eq!(
        (s, &j["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "role": "too_high" })
        )
    );
    let (s, _) = invite(&app, As::Bearer(&admin), "new@example.com", "admin").await;
    assert_eq!(s, StatusCode::CREATED);

    // Below admin: forbidden on every invitation route.
    let (s, j) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/invites",
        As::Bearer(&mem),
        None,
    )
    .await;
    assert_eq!(
        (s, j),
        (StatusCode::FORBIDDEN, json!({ "code": "forbidden" }))
    );
    assert_eq!(
        invite(&app, As::Bearer(&mem), "x@example.com", "accountant")
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // An owner invitation is an owner's business: an admin may not resend,
    // revoke or replace it.
    let (s, o) = invite(&app, OWNER, "boss@example.com", "owner").await;
    assert_eq!(s, StatusCode::CREATED);
    let id = o["id"].as_str().expect("id");
    let resend = format!("/api/invites/{id}/resend");
    let revoke = format!("/api/invites/{id}");
    assert_eq!(
        on(
            &app,
            Method::POST,
            TEST_HOST,
            &resend,
            As::Bearer(&admin),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        on(
            &app,
            Method::DELETE,
            TEST_HOST,
            &revoke,
            As::Bearer(&admin),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        invite(&app, As::Bearer(&admin), "boss@example.com", "member")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        show(&app, &link_token(&o["url"])).await.0,
        StatusCode::OK,
        "untouched"
    );
}

#[tokio::test]
async fn another_space_invitations_are_not_found() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (beta_token, _) = invoiceapp::auth::crypto::new_api_token().expect("token");
    seed_space(&db.conn, "beta", "beta@example.com", Some(&beta_token)).await;
    let body = json!({ "email": "nova@example.com", "role": "member" });
    let (s, b) = on(
        &app,
        Method::POST,
        B_HOST,
        "/api/invites",
        As::Bearer(&beta_token),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(
        b["url"]
            .as_str()
            .unwrap_or_default()
            .starts_with("http://beta.localhost:3000/invite?token=")
    );
    let id = b["id"].as_str().expect("id");
    let resend = format!("/api/invites/{id}/resend");
    let revoke = format!("/api/invites/{id}");
    assert_eq!(
        on(&app, Method::POST, TEST_HOST, &resend, OWNER, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        on(&app, Method::DELETE, TEST_HOST, &revoke, OWNER, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (_, list) = on(&app, Method::GET, TEST_HOST, "/api/invites", OWNER, None).await;
    assert_eq!(list, json!([]));
    // Beta's link on acme's host (or the base host) is unknown.
    let token = link_token(&b["url"]);
    assert_eq!(show(&app, &token).await.0, StatusCode::NOT_FOUND);
    let uri = format!("/api/invites/accept?token={token}");
    assert_eq!(
        on(&app, Method::GET, common::BASE_HOST, &uri, As::Nobody, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        on(&app, Method::GET, B_HOST, &uri, As::Nobody, None)
            .await
            .0,
        StatusCode::OK
    );
}
