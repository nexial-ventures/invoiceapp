//! The role matrix per route group (incl. MCP tools), the token role
//! ceiling and the personal API token routes (spaces.md, 4a). Every route's
//! access decision is listed and probed in `spaces_route_roles.rs`.

mod common;

use axum::http::{Method, StatusCode};
use common::auth::{As, member, on, status, store_token};
use common::mcp::tool_as;
use common::{TEST_HOST, TEST_TOKEN, TestDb, router};
use invoiceapp::space::Role;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

fn company() -> Value {
    json!({ "name": "Acme s.r.o.", "ico": "12345679", "vatPayer": false, "street": "", "city": "",
            "zip": "", "country": "CZ", "defaultDueDays": 14, "defaultLocale": "cs" })
}

/// (min role, method, uri, body, success status)
fn matrix(doc: &str) -> Vec<(Role, Method, String, Option<Value>, StatusCode)> {
    let ok = StatusCode::OK;
    vec![
        (
            Role::Accountant,
            Method::GET,
            "/api/documents?direction=issued".into(),
            None,
            ok,
        ),
        (
            Role::Accountant,
            Method::GET,
            format!("/api/documents/{doc}"),
            None,
            ok,
        ),
        (
            Role::Accountant,
            Method::GET,
            "/api/contacts".into(),
            None,
            ok,
        ),
        (
            Role::Accountant,
            Method::GET,
            "/api/settings/company".into(),
            None,
            ok,
        ),
        (
            Role::Accountant,
            Method::GET,
            "/api/export/csv?direction=issued".into(),
            None,
            ok,
        ),
        (
            Role::Accountant,
            Method::GET,
            "/api/export/accountant?from=2026-01-01&to=2026-12-31".into(),
            None,
            ok,
        ),
        (Role::Accountant, Method::GET, "/api/space".into(), None, ok),
        (
            Role::Member,
            Method::POST,
            "/api/contacts".into(),
            Some(json!({ "name": "C" })),
            StatusCode::CREATED,
        ),
        (
            Role::Member,
            Method::POST,
            "/api/documents/compute".into(),
            Some(json!({ "lines": [] })),
            ok,
        ),
        (
            Role::Member,
            Method::PUT,
            format!("/api/documents/{doc}/internal-note"),
            Some(json!({ "internalNote": "x" })),
            ok,
        ),
        (
            Role::Member,
            Method::POST,
            "/api/catalog/items".into(),
            Some(json!({ "name": "I", "unitPrice": "1", "currency": "CZK", "vatRate": "21" })),
            StatusCode::CREATED,
        ),
        (
            Role::Admin,
            Method::PUT,
            "/api/settings/company".into(),
            Some(company()),
            ok,
        ),
        (
            Role::Admin,
            Method::POST,
            "/api/settings/categories".into(),
            Some(json!({ "name": "K", "kind": "expense" })),
            StatusCode::CREATED,
        ),
        (
            Role::Admin,
            Method::PUT,
            "/api/settings/accounting".into(),
            Some(json!({})),
            ok,
        ),
        (
            Role::Admin,
            Method::PUT,
            "/api/space".into(),
            Some(json!({ "name": "Acme" })),
            ok,
        ),
    ]
}

#[tokio::test]
async fn role_matrix_per_route_group() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    common::documents::set_company(&app, true).await;
    let doc = common::documents::id(
        &common::documents::create_doc(
            &app,
            json!({ "lines": [common::documents::item("1", "100", "21")] }),
        )
        .await,
    );
    let mut tokens = vec![(Role::Owner, TEST_TOKEN.to_string())];
    for (role, email) in [
        (Role::Accountant, "acc@example.com"),
        (Role::Member, "mem@example.com"),
        (Role::Admin, "adm@example.com"),
    ] {
        tokens.push((role, member(&db.conn, db.space, email, role).await));
    }
    for (min, m, uri, body, success) in matrix(&doc) {
        for (role, token) in &tokens {
            // Category names are unique per space: one per caller.
            let body = match &body {
                Some(b) if uri.ends_with("/categories") => {
                    Some(json!({ "name": format!("K {role:?}"), "kind": b["kind"] }))
                }
                other => other.clone(),
            };
            let got = status(&app, m.clone(), &uri, token, body).await;
            let want = if *role >= min {
                success
            } else {
                StatusCode::FORBIDDEN
            };
            assert_eq!(got, want, "{role:?} {m} {uri}");
        }
    }
    // Owner only: deleting the space (also session only, see spaces.rs).
    let admin = &tokens[3].1;
    let (s, _) = on(
        &app,
        Method::DELETE,
        TEST_HOST,
        "/api/space",
        As::Bearer(admin),
        Some(json!({ "slug": "acme", "password": "x" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn mcp_tools_by_role() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let accountant = member(&db.conn, db.space, "acc@example.com", Role::Accountant).await;
    let mem = member(&db.conn, db.space, "mem@example.com", Role::Member).await;

    let read = tool_as(&app, TEST_HOST, &accountant, "list_contacts", json!({})).await;
    assert_eq!(read["isError"], false, "{read}");
    let settings = tool_as(&app, TEST_HOST, &accountant, "get_settings", json!({})).await;
    assert_eq!(settings["isError"], false, "{settings}");
    for (name, args) in [
        ("create_contact", json!({ "name": "C" })),
        ("create_draft", json!({ "lines": [] })),
        ("issue_document", json!({ "id": uuid::Uuid::nil() })),
        (
            "add_payment",
            json!({ "id": uuid::Uuid::nil(), "date": "2026-10-01", "amount": "1" }),
        ),
        ("mark_sent", json!({ "id": uuid::Uuid::nil() })),
        (
            "update_draft",
            json!({ "id": uuid::Uuid::nil(), "lines": [] }),
        ),
    ] {
        let r = tool_as(&app, TEST_HOST, &accountant, name, args).await;
        assert_eq!(r["isError"], true, "{name}");
        assert_eq!(
            r["structuredContent"],
            json!({ "code": "forbidden" }),
            "{name}"
        );
    }
    let w = tool_as(
        &app,
        TEST_HOST,
        &mem,
        "create_contact",
        json!({ "name": "C" }),
    )
    .await;
    assert_eq!(w["isError"], false, "{w}");
}

#[tokio::test]
async fn token_role_is_capped_by_the_membership() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    // An accountant token of the owner: read only.
    let (low, _) = invoiceapp::auth::crypto::new_api_token().expect("token");
    store_token(&db.conn, db.space, db.user_id, Role::Accountant, &low).await;
    assert_eq!(
        status(&app, Method::GET, "/api/contacts", &low, None).await,
        StatusCode::OK
    );
    assert_eq!(
        status(
            &app,
            Method::POST,
            "/api/contacts",
            &low,
            Some(json!({ "name": "C" }))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let (_, me) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/auth/me",
        As::Bearer(&low),
        None,
    )
    .await;
    assert_eq!(me["space"]["role"], "accountant");

    // An admin token whose membership drops to member acts as a member.
    let admin = member(&db.conn, db.space, "adm@example.com", Role::Admin).await;
    assert_eq!(
        status(
            &app,
            Method::PUT,
            "/api/settings/company",
            &admin,
            Some(company())
        )
        .await,
        StatusCode::OK
    );
    db.conn
        .execute_unprepared("UPDATE space_members SET role = 'member' WHERE role = 'admin'")
        .await
        .expect("downgrade");
    assert_eq!(
        status(
            &app,
            Method::PUT,
            "/api/settings/company",
            &admin,
            Some(company())
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(
            &app,
            Method::POST,
            "/api/contacts",
            &admin,
            Some(json!({ "name": "D" }))
        )
        .await,
        StatusCode::CREATED
    );
    // No membership → 401.
    db.conn
        .execute_unprepared("DELETE FROM space_members WHERE role = 'member'")
        .await
        .expect("remove");
    assert_eq!(
        status(&app, Method::GET, "/api/contacts", &admin, None).await,
        StatusCode::UNAUTHORIZED
    );
}
