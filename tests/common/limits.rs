//! Hidden space limits (spaces.md, "Limits"): what an operator changes with
//! SQL, and the usage `GET /api/space` reports.

use axum::Router;
use axum::http::{Method, StatusCode};
use invoiceapp::space::SpaceId;
use sea_orm::{ConnectionTrait, DatabaseConnection};
use serde_json::Value;

pub const KIB: usize = 1024;
pub const MIB: u64 = 1024 * 1024;

/// The operator statement of spaces.md.
pub async fn set_quota(conn: &DatabaseConnection, space: SpaceId, bytes: u64) {
    conn.execute_unprepared(&format!(
        "UPDATE space_limits SET storage_quota_bytes = {bytes}, updated_at = now() \
         WHERE space_id = '{space}'"
    ))
    .await
    .expect("set the storage quota");
}

/// `limits` of `GET /api/space` on the default space.
pub async fn limits(app: &Router) -> Value {
    let (status, body) = super::call(app, Method::GET, "/api/space", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["limits"].clone()
}

pub async fn used(app: &Router) -> u64 {
    limits(app).await["storageUsedBytes"]
        .as_u64()
        .expect("storageUsedBytes")
}

/// A PDF (as far as the upload check goes) of exactly `size` bytes.
pub fn pdf_of(size: usize) -> Vec<u8> {
    let mut b = b"%PDF-1.4\n".to_vec();
    b.resize(size, b' ');
    b
}
