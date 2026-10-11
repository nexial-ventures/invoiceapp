//! Personal API tokens (`/api/tokens`, space host, session or token).

pub mod repo;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Extension, Json};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::ctx::{Authed, Read, Via};
use crate::auth::entity::{api_token, user};
use crate::auth::host::HostCtx;
use crate::auth::mfa;
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::{ApiJson, ApiPath};
use crate::space::Role;
use crate::validation as v;

/// Routes relative to `/api/tokens`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", delete(revoke))
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenUser {
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub role: Role,
    pub created_at: DateTime<FixedOffset>,
    /// Last valid day (UTC), `YYYY-MM-DD`.
    pub expires_at: Option<NaiveDate>,
    pub last_used_at: Option<DateTime<FixedOffset>>,
    /// Admin+ listings only: whose token it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<TokenUser>,
}

impl ApiToken {
    fn from_row(m: api_token::Model, owner: Option<user::Model>) -> Self {
        Self {
            id: m.id,
            name: m.name,
            prefix: m.prefix,
            role: Role::parse(&m.role).unwrap_or(Role::Accountant),
            created_at: m.created_at,
            expires_at: m.expires_at,
            last_used_at: m.last_used_at,
            user: owner.map(|u| TokenUser {
                email: u.email,
                display_name: u.display_name,
            }),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedToken {
    #[serde(flatten)]
    pub token_info: ApiToken,
    /// The token itself — shown only in this response.
    pub token: String,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TokenInput {
    pub name: String,
    pub role: Option<String>,
    /// `YYYY-MM-DD`, today or later; null = no expiry.
    pub expires_at: Option<String>,
    /// Users with TOTP: a TOTP or recovery code.
    pub code: Option<String>,
}

impl TokenInput {
    /// `cap`: the expiry of the token the caller authenticated with — a
    /// token may not mint one that outlives it (`expiresAt: too_late`, also
    /// for no expiry). `None` = no cap (a session, or a token without expiry).
    pub fn validate(
        self,
        caller: Role,
        today: NaiveDate,
        cap: Option<NaiveDate>,
    ) -> Result<repo::NewToken, AppError> {
        let mut e = FieldErrors::new();
        let name = e.check("name", v::required_text(&self.name, 100));
        let role = e.check(
            "role",
            Role::from_input(self.role.as_deref())
                .and_then(|r| if r > caller { Err("too_high") } else { Ok(r) }),
        );
        let expires_at = match self.expires_at.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(s) => match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                Ok(d) if d >= today => Some(d),
                _ => {
                    e.add("expiresAt", "invalid");
                    None
                }
            },
        };
        if let Some(cap) = cap
            && e.get("expiresAt").is_none()
            && expires_at.is_none_or(|d| d > cap)
        {
            e.add("expiresAt", "too_late");
        }
        e.into_result()?;
        Ok(repo::NewToken {
            name: name.unwrap_or_default(),
            role: role.unwrap_or(Role::Accountant),
            expires_at,
        })
    }
}

/// Own tokens; admin+ see every token of the space with its user.
#[utoipa::path(
    get,
    path = "/api/tokens",
    tag = "tokens",
    security(("cookie" = []), ("bearer" = [])),
    responses((status = 200, body = Vec<ApiToken>))
)]
pub async fn list(
    State(state): State<AppState>,
    access: Read,
) -> Result<Json<Vec<ApiToken>>, AppError> {
    let s = access.scope;
    let admin = s.role >= Role::Admin;
    let owner = (!admin).then_some(s.user_id);
    let rows = repo::list(&state.db, s.space, owner).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(t, u)| ApiToken::from_row(t, u.filter(|_| admin)))
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/tokens",
    tag = "tokens",
    security(("cookie" = []), ("bearer" = [])),
    request_body = TokenInput,
    responses(
        (status = 201, body = CreatedToken),
        (status = 403, description = "`mfa_required`: the space requires TOTP and the caller has none", body = ErrorBody),
        (status = 422, description = "Validation failed (`role: too_high`, `expiresAt: invalid | too_late` (after the calling token's expiry), …; `code: required | invalid` for a user with TOTP)", body = ErrorBody),
        (status = 429, description = "`rate_limited` (login bucket, code checks)", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    access: Read,
    authed: Authed,
    Extension(host): Extension<HostCtx>,
    ApiJson(mut input): ApiJson<TokenInput>,
) -> Result<(StatusCode, Json<CreatedToken>), AppError> {
    let s = access.scope;
    // Minting a credential counts as a new login: the space's policy applies.
    if host.requires_mfa() && !authed.mfa_enabled {
        return Err(AppError::MfaRequired {
            account_created: false,
        });
    }
    let code = input.code.take();
    let mfa_on = authed.mfa_enabled;
    let cap = match authed.via {
        Via::Token(id) => repo::expiry(&state.db, id).await?,
        Via::Session(_) => None,
    };
    let new = match input.validate(s.role, Utc::now().date_naive(), cap) {
        Ok(new) => new,
        Err(AppError::Validation(mut e)) => {
            // Adds only `code: required` here (nothing is spent on a failing request).
            mfa::step_up(&state, &authed.user, mfa_on, code.as_deref(), &mut e).await?;
            return Err(AppError::Validation(e));
        }
        Err(other) => return Err(other),
    };
    let mut e = FieldErrors::new();
    mfa::step_up(&state, &authed.user, mfa_on, code.as_deref(), &mut e).await?;
    e.into_result()?;
    let (row, token) = repo::create(&state.db, s.space, s.user_id, new).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedToken {
            token_info: ApiToken::from_row(row, None),
            token,
        }),
    ))
}

/// Own token, or any token of the space for admin+; else 404.
#[utoipa::path(
    delete,
    path = "/api/tokens/{id}",
    tag = "tokens",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ErrorBody))
)]
pub async fn revoke(
    State(state): State<AppState>,
    access: Read,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    let s = access.scope;
    let owner = (s.role < Role::Admin).then_some(s.user_id);
    if !repo::delete(&state.db, s.space, id, owner).await? {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 10).expect("date")
    }

    fn input(name: &str, role: &str, exp: Option<&str>) -> TokenInput {
        TokenInput {
            name: name.into(),
            role: Some(role.into()),
            expires_at: exp.map(str::to_string),
            code: None,
        }
    }

    fn reasons(err: AppError) -> String {
        format!("{err:?}")
    }

    #[test]
    fn validates_tokens() {
        let ok = input(" MCP ", "member", Some("2026-10-10"))
            .validate(Role::Member, today(), None)
            .expect("valid");
        assert_eq!(ok.name, "MCP");
        assert_eq!(ok.role, Role::Member);
        assert_eq!(ok.expires_at, Some(today()));
        let err = input("x", "admin", None)
            .validate(Role::Member, today(), None)
            .expect_err("too high");
        assert!(reasons(err).contains("\"role\": \"too_high\""));
        let err = input("", "boss", Some("2026-10-09"))
            .validate(Role::Owner, today(), None)
            .expect_err("invalid");
        let r = reasons(err);
        assert!(r.contains("\"name\": \"required\""), "{r}");
        assert!(r.contains("\"role\": \"invalid\""), "{r}");
        assert!(r.contains("\"expiresAt\": \"invalid\""), "{r}");
        let err = TokenInput {
            name: "x".repeat(101),
            ..Default::default()
        }
        .validate(Role::Owner, today(), None)
        .expect_err("invalid");
        let r = reasons(err);
        assert!(r.contains("\"name\": \"too_long\""), "{r}");
        assert!(r.contains("\"role\": \"required\""), "{r}");
    }

    #[test]
    fn a_token_cannot_outlive_its_minter() {
        let cap = NaiveDate::from_ymd_opt(2026, 10, 20);
        let ok = |exp| input("x", "member", exp).validate(Role::Member, today(), cap);
        assert!(ok(Some("2026-10-20")).is_ok());
        assert!(ok(Some("2026-10-12")).is_ok());
        for late in [Some("2026-10-21"), None, Some("")] {
            let r = reasons(ok(late).expect_err("too late"));
            assert!(r.contains("\"expiresAt\": \"too_late\""), "{r}");
        }
        // A past date stays `invalid`.
        let r = reasons(ok(Some("2026-10-01")).expect_err("past"));
        assert!(r.contains("\"expiresAt\": \"invalid\""), "{r}");
    }
}
