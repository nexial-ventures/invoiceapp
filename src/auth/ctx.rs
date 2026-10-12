//! Who is calling: the session cookie or a personal Bearer token resolved to
//! a user (and, on a space host, the space and the effective role). The
//! `authenticate` middleware puts [`Authed`] into the request extensions;
//! handlers take it through the extractors below, which enforce the minimum
//! role per route (403 `forbidden`).

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

use super::entity::user;
use super::host::HostCtx;
use super::{crypto, resolve, session};
use crate::app::AppState;
use crate::error::AppError;
use crate::space::{Role, SpaceId};

/// How the request authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Session(Uuid),
    Token(Uuid),
}

/// The space side of an authenticated request on a space host.
#[derive(Debug, Clone)]
pub struct Membership {
    pub space: SpaceId,
    pub slug: String,
    pub name: String,
    /// Effective role: the membership role (session) or
    /// `min(token role, membership role)` (token).
    pub role: Role,
}

#[derive(Debug, Clone)]
pub struct Authed {
    pub user: user::Model,
    /// `None` on the base host.
    pub space: Option<Membership>,
    pub via: Via,
    /// The user has TOTP on (`docs/api/mfa.md`).
    pub mfa_enabled: bool,
}

impl Authed {
    /// 403 for a token where the contract wants an interactive session.
    pub fn require_session(&self) -> Result<Uuid, AppError> {
        match self.via {
            Via::Session(id) => Ok(id),
            Via::Token(_) => Err(AppError::Forbidden),
        }
    }

    /// 403 `email_unverified` until the user proved the address: credentials
    /// set up before that (password, TOTP, Google link) could belong to
    /// whoever registered someone else's address.
    pub fn require_verified(&self) -> Result<(), AppError> {
        match self.user.email_verified_at {
            Some(_) => Ok(()),
            None => Err(AppError::EmailUnverified),
        }
    }

    /// The space scope at `min` role or above.
    pub fn scope(&self, min: Role) -> Result<Scope, AppError> {
        let m = self.space.as_ref().ok_or(AppError::NotFound)?;
        if m.role < min {
            return Err(AppError::Forbidden);
        }
        Ok(Scope {
            space: m.space,
            user_id: self.user.id,
            role: m.role,
            via: self.via,
        })
    }
}

/// What a space route works with.
#[derive(Debug, Clone, Copy)]
pub struct Scope {
    pub space: SpaceId,
    pub user_id: Uuid,
    pub role: Role,
    pub via: Via,
}

fn is_mutation(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The CSRF rule: when `Origin` is present it must equal the request's own
/// origin; `required` (cookie-authenticated mutations) refuses a missing one.
pub fn check_origin(
    parts_headers: &axum::http::HeaderMap,
    host: &HostCtx,
    required: bool,
) -> Result<(), AppError> {
    match parts_headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        Some(origin) if origin.eq_ignore_ascii_case(&host.origin) => Ok(()),
        Some(_) => Err(AppError::Csrf),
        None if required => Err(AppError::Csrf),
        None => Ok(()),
    }
}

/// Resolve the caller or answer 401 (403 `csrf` for a cookie mutation
/// without the right `Origin`).
pub async fn authenticate(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let host = req
        .extensions()
        .get::<HostCtx>()
        .cloned()
        .ok_or(AppError::NotFound)?;
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(crypto::parse_bearer)
        .map(str::to_string);
    let authed = match bearer {
        Some(token) => {
            check_origin(req.headers(), &host, false)?;
            by_token(&state, &host, &token).await?
        }
        None => {
            let cookie = session::cookie_value(req.headers()).ok_or(AppError::Unauthorized)?;
            let authed = by_session(&state, &host, &cookie).await?;
            check_origin(req.headers(), &host, is_mutation(req.method()))?;
            authed
        }
    };
    req.extensions_mut().insert(authed);
    Ok(next.run(req).await)
}

async fn by_token(state: &AppState, host: &HostCtx, token: &str) -> Result<Authed, AppError> {
    // A token works only on its own space's host.
    let space = host.space.as_ref().ok_or(AppError::Unauthorized)?;
    let found = resolve::token(&state.db, token, SpaceId(space.id))
        .await?
        .ok_or(AppError::Unauthorized)?;
    let token_role = found.token_role.ok_or(AppError::Unauthorized)?;
    let mfa_enabled = found.mfa_enabled;
    let (user, member_role) = member(found.user, found.member_role)?;
    Ok(Authed {
        user,
        space: Some(Membership {
            space: SpaceId(space.id),
            slug: space.slug.clone(),
            name: space.name.clone(),
            role: token_role.min(member_role),
        }),
        via: Via::Token(found.id),
        mfa_enabled,
    })
}

pub(crate) async fn by_session(
    state: &AppState,
    host: &HostCtx,
    cookie: &str,
) -> Result<Authed, AppError> {
    let found = resolve::session(&state.db, cookie, host.space_id())
        .await?
        .ok_or(AppError::Unauthorized)?;
    let Some(space) = host.space.as_ref() else {
        if found.user.disabled {
            return Err(AppError::Unauthorized);
        }
        return Ok(Authed {
            user: found.user,
            space: None,
            via: Via::Session(found.id),
            mfa_enabled: found.mfa_enabled,
        });
    };
    let mfa_enabled = found.mfa_enabled;
    let (user, role) = member(found.user, found.member_role)?;
    Ok(Authed {
        user,
        space: Some(Membership {
            space: SpaceId(space.id),
            slug: space.slug.clone(),
            name: space.name.clone(),
            role,
        }),
        via: Via::Session(found.id),
        mfa_enabled,
    })
}

/// An active, verified member, else 401.
fn member(user: user::Model, role: Option<Role>) -> Result<(user::Model, Role), AppError> {
    match role {
        Some(r) if !user.disabled && user.email_verified_at.is_some() => Ok((user, r)),
        _ => Err(AppError::Unauthorized),
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Authed {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Authed>()
            .cloned()
            .ok_or(AppError::Unauthorized)
    }
}

/// A space route at minimum role `R` (0 accountant, 1 member, 2 admin,
/// 3 owner). Use the aliases [`Read`], [`Write`], [`Manage`], [`Own`].
#[derive(Debug, Clone, Copy)]
pub struct Access<const R: u8> {
    pub scope: Scope,
}

impl<const R: u8> Access<R> {
    pub fn space(&self) -> SpaceId {
        self.scope.space
    }

    const fn min_role() -> Role {
        match R {
            0 => Role::Accountant,
            1 => Role::Member,
            2 => Role::Admin,
            _ => Role::Owner,
        }
    }
}

/// GET routes and exports: any role.
pub type Read = Access<0>;
/// Mutations of business data (and read-only POSTs): member+.
pub type Write = Access<1>;
/// Settings mutations: admin+.
pub type Manage = Access<2>;
/// Owner only.
pub type Own = Access<3>;

impl<S: Send + Sync, const R: u8> FromRequestParts<S> for Access<R> {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let authed = Authed::from_request_parts(parts, state).await?;
        Ok(Self {
            scope: authed.scope(Self::min_role())?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn host() -> HostCtx {
        HostCtx {
            space: None,
            origin: "https://firma.invoiceapp.cz".into(),
        }
    }

    #[test]
    fn origin_rule() {
        let mut h = HeaderMap::new();
        assert!(check_origin(&h, &host(), false).is_ok());
        assert!(matches!(
            check_origin(&h, &host(), true),
            Err(AppError::Csrf)
        ));
        h.insert(
            header::ORIGIN,
            "https://Firma.invoiceapp.cz".parse().unwrap(),
        );
        assert!(check_origin(&h, &host(), true).is_ok());
        h.insert(header::ORIGIN, "https://evil.example.com".parse().unwrap());
        assert!(matches!(
            check_origin(&h, &host(), false),
            Err(AppError::Csrf)
        ));
        h.insert(header::ORIGIN, "null".parse().unwrap());
        assert!(matches!(
            check_origin(&h, &host(), true),
            Err(AppError::Csrf)
        ));
    }

    #[test]
    fn mutations() {
        assert!(!is_mutation(&Method::GET));
        assert!(!is_mutation(&Method::HEAD));
        assert!(is_mutation(&Method::POST));
        assert!(is_mutation(&Method::DELETE));
        assert!(is_mutation(&Method::PUT));
    }

    #[test]
    fn min_roles() {
        assert_eq!(Read::min_role(), Role::Accountant);
        assert_eq!(Write::min_role(), Role::Member);
        assert_eq!(Manage::min_role(), Role::Admin);
        assert_eq!(Own::min_role(), Role::Owner);
    }
}
