//! Spaces (one space = one own company = one accounting unit), memberships
//! and roles. Contract: `docs/api/spaces.md`.
//!
//! Every repo function on space data takes a [`SpaceId`]; it only ever comes
//! from the request's auth context (`auth::Scope`), never from client input.

pub mod entity;
pub mod handlers;
pub mod limits;
pub mod repo;
pub mod seed;
pub mod slug;

use std::fmt;

use axum::Router;
use axum::routing::{get, post};
use sea_orm::Value;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::app::AppState;

/// The space a request works in. Deliberately not `Deserialize`: it cannot
/// be taken from a request body or query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpaceId(pub Uuid);

impl SpaceId {
    pub fn uuid(self) -> Uuid {
        self.0
    }

    /// The storage prefix every key of the space lives under.
    pub fn storage_prefix(self) -> String {
        format!("spaces/{}", self.0)
    }
}

impl fmt::Display for SpaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<SpaceId> for Value {
    fn from(id: SpaceId) -> Self {
        id.0.into()
    }
}

/// Membership / token role, ordered by what it may do.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Accountant,
    Member,
    Admin,
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accountant => "accountant",
            Self::Member => "member",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    /// The `role` field of a request body: `required` | `invalid`.
    pub fn from_input(raw: Option<&str>) -> Result<Self, &'static str> {
        match raw.map(str::trim) {
            None | Some("") => Err("required"),
            Some(r) => Self::parse(r).ok_or("invalid"),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "accountant" => Some(Self::Accountant),
            "member" => Some(Self::Member),
            "admin" => Some(Self::Admin),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }
}

/// Base-host routes relative to `/api/spaces`.
pub fn base_router() -> Router<AppState> {
    Router::new().route("/", get(handlers::list).post(handlers::create))
}

/// Space-host routes relative to `/api/space`.
pub fn space_router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(handlers::get)
                .put(handlers::update)
                .delete(handlers::delete),
        )
        .route("/leave", post(crate::members::handlers::leave))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_ordered_and_round_trip() {
        assert!(Role::Accountant < Role::Member);
        assert!(Role::Member < Role::Admin);
        assert!(Role::Admin < Role::Owner);
        for r in [Role::Accountant, Role::Member, Role::Admin, Role::Owner] {
            assert_eq!(Role::parse(r.as_str()), Some(r));
        }
        assert_eq!(Role::parse("root"), None);
        assert_eq!(Role::from_input(Some(" admin ")), Ok(Role::Admin));
        assert_eq!(Role::from_input(None), Err("required"));
        assert_eq!(Role::from_input(Some("")), Err("required"));
        assert_eq!(Role::from_input(Some("boss")), Err("invalid"));
        assert_eq!(Role::Accountant.min(Role::Owner), Role::Accountant);
    }

    #[test]
    fn storage_prefix() {
        let id = SpaceId(Uuid::nil());
        assert_eq!(
            id.storage_prefix(),
            "spaces/00000000-0000-0000-0000-000000000000"
        );
    }
}
