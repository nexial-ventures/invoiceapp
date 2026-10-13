//! Members and invitations (members.md, 4b): direct seeding of memberships,
//! sessions and tokens in any space, and the invite / accept calls.

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use invoiceapp::auth::{crypto, session, users};
use invoiceapp::space::{Role, SpaceId};
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use super::TEST_HOST;
use super::auth::{As, exchange, on, request, session_cookie, store_token};

/// Space beta's host (a second space for isolation checks).
pub const B_HOST: &str = "beta.localhost:3000";
/// The token `seed_space(.., "beta", .., Some(B_TOKEN))` stores for beta's owner.
pub const B_TOKEN: &str = "inv_0b0b0b0b_YmV0YS1zcGFjZS10b2tlbi1zZWNyZXQtMDEyMzQ1Njc4OWFi";

pub async fn user_id(conn: &DatabaseConnection, email: &str) -> Uuid {
    users::find_by_email(conn, email)
        .await
        .expect("query user")
        .expect("user exists")
        .id
}

/// Add an existing user to `space` with `role`.
pub async fn join(conn: &DatabaseConnection, space: SpaceId, user: Uuid, role: Role) {
    invoiceapp::space::entity::member::ActiveModel {
        space_id: Set(space.uuid()),
        user_id: Set(user),
        role: Set(role.as_str().into()),
        created_at: Set(chrono::Utc::now().into()),
    }
    .insert(conn)
    .await
    .expect("add member");
}

/// A new API token of `user` in `space` with `role`.
pub async fn token(conn: &DatabaseConnection, space: SpaceId, user: Uuid, role: Role) -> String {
    let (t, _) = crypto::new_api_token().expect("token");
    store_token(conn, space, user, role, &t).await;
    t
}

/// A session of `user` on `space`'s host (no login, no rate limit).
pub async fn session_on(conn: &DatabaseConnection, space: SpaceId, user: Uuid) -> String {
    session::create(conn, user, Some(space), None, session::Method::Password)
        .await
        .expect("create session")
}

/// A single `count(*)` query.
pub async fn count(conn: &DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one(sea_orm::Statement::from_string(
            conn.get_database_backend(),
            sql.to_string(),
        ))
        .await
        .expect("count query")
        .expect("a row");
    row.try_get_by_index::<i64>(0).expect("count")
}

/// `POST /api/invites` on the default space as `who`.
pub async fn invite(app: &Router, who: As<'_>, email: &str, role: &str) -> (StatusCode, Value) {
    let body = json!({ "email": email, "role": role });
    on(
        app,
        Method::POST,
        TEST_HOST,
        "/api/invites",
        who,
        Some(body),
    )
    .await
}

/// The token of an invitation `url` (`…/invite?token=…`).
pub fn link_token(url: &Value) -> String {
    let url = url.as_str().expect("url");
    url.split_once("/invite?token=")
        .map(|(_, t)| t.to_string())
        .unwrap_or_else(|| panic!("no invite link: {url}"))
}

/// Invite `email` and return the link token.
pub async fn invited(app: &Router, email: &str, role: &str) -> String {
    let (s, j) = invite(app, As::Bearer(super::TEST_TOKEN), email, role).await;
    assert_eq!(s, StatusCode::CREATED, "{j}");
    link_token(&j["url"])
}

/// `POST /api/invites/accept` on `host` (with the host's own `Origin`, as
/// the SPA sends it); the status, the session cookie set and the body.
pub async fn accept(
    app: &Router,
    host: &str,
    body: Value,
) -> (StatusCode, Option<String>, Value, HeaderMap) {
    let req = request(
        Method::POST,
        host,
        "/api/invites/accept",
        As::Nobody,
        Some(body.clone()),
    )
    .header(header::ORIGIN, super::auth::origin(host))
    .body(Body::from(body.to_string()))
    .expect("build request");
    let (s, headers, json) = exchange(app, req).await;
    (s, session_cookie(&headers), json, headers)
}

/// `GET /api/invites/accept?token=…` on the default space.
pub async fn show(app: &Router, token: &str) -> (StatusCode, Value) {
    let uri = format!("/api/invites/accept?token={token}");
    on(app, Method::GET, TEST_HOST, &uri, As::Nobody, None).await
}

/// `app` as seen by space beta: every request goes to beta's host with
/// beta's token, so the shared helpers (which use the default host and
/// token) act in beta.
pub fn as_beta(app: &Router) -> Router {
    app.clone().layer(axum::middleware::map_request(
        |mut req: Request| async move {
            let h = req.headers_mut();
            h.insert(header::HOST, HeaderValue::from_static(B_HOST));
            if h.contains_key(header::AUTHORIZATION) {
                let bearer = HeaderValue::from_str(&format!("Bearer {B_TOKEN}")).expect("header");
                h.insert(header::AUTHORIZATION, bearer);
            }
            req
        },
    ))
}
