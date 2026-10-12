//! The outbound e-mail limits (`email_sends`, spaces.md "Limits"): seeding
//! past sends directly and sending with the `Retry-After` header.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde_json::Value;
use uuid::Uuid;

use super::TEST_HOST;
use super::auth::{As, exchange, request};

/// `n` past sends of `user` (in `space`, `None` = a deleted space), each
/// `ago` (a Postgres interval) before now.
pub async fn seed_sends(
    conn: &DatabaseConnection,
    space: Option<Uuid>,
    user: Uuid,
    n: i32,
    ago: &str,
) {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "INSERT INTO email_sends (space_id, user_id, kind, created_at) \
         SELECT $1, $2, 'document', now() - $3::interval FROM generate_series(1, $4)",
        [space.into(), user.into(), ago.into(), n.into()],
    ))
    .await
    .expect("seed email_sends");
}

/// Move every recorded send `by` (a Postgres interval) into the past.
pub async fn age_sends(conn: &DatabaseConnection, by: &str) {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "UPDATE email_sends SET created_at = created_at - $1::interval",
        [by.into()],
    ))
    .await
    .expect("age email_sends");
}

/// `email_sends` rows of `kind` (`None` = all).
pub async fn sends(conn: &DatabaseConnection, kind: Option<&str>) -> i64 {
    let filter = kind.map_or(String::new(), |k| format!(" WHERE kind = '{k}'"));
    super::members::count(conn, &format!("SELECT count(*) FROM email_sends{filter}")).await
}

/// `POST` `body` to `uri` on the default space as `who`: the status, the
/// `Retry-After` seconds (when sent) and the body.
pub async fn post(
    app: &Router,
    who: As<'_>,
    uri: &str,
    body: Value,
) -> (StatusCode, Option<u64>, Value) {
    let req = request(Method::POST, TEST_HOST, uri, who, Some(body.clone()))
        .body(Body::from(body.to_string()))
        .expect("build request");
    let (status, headers, json) = exchange(app, req).await;
    let retry = headers
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    (status, retry, json)
}

/// A 429 `rate_limited` whose `Retry-After` is within `secs` ± 10.
pub fn assert_limited(got: (StatusCode, Option<u64>, Value), secs: u64) {
    let (status, retry, body) = got;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body, serde_json::json!({ "code": "rate_limited" }));
    let retry = retry.expect("Retry-After");
    assert!(
        retry.abs_diff(secs) <= 10,
        "Retry-After {retry}, expected about {secs}"
    );
}
