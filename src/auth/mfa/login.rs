//! The login's code step: the `invoice_mfa` pending cookie (host-only,
//! 5 min, single use, sha256 stored with the user and the host) and
//! `POST /api/auth/login/mfa`.

use axum::Extension;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::ToSchema;

use sea_orm::TransactionTrait;

use crate::auth::session::Method;
use crate::members::join::{Code, Joined, Joiner, join};

use super::repo;
use super::verify::{self, Verifier};
use crate::app::AppState;
use crate::auth::handlers::ClientIp;
use crate::auth::handlers::login::{may_sign_in, with_cookies};
use crate::auth::host::HostCtx;
use crate::auth::ratelimit::{LOGIN_EMAIL, LOGIN_IP};
use crate::auth::{session, users};
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiJson;

pub const PENDING_COOKIE: &str = "invoice_mfa";

/// `Set-Cookie` of a pending login (attributes as the session cookie).
pub fn pending_cookie(value: &str, secure: bool) -> String {
    session::cookie(PENDING_COOKIE, value, repo::PENDING_LOGIN, secure)
}

/// `Set-Cookie` that removes the pending cookie.
pub fn clear_pending(secure: bool) -> String {
    session::cookie(PENDING_COOKIE, "", chrono::Duration::zero(), secure)
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct CodeInput {
    /// TOTP code or recovery code.
    pub code: String,
}

/// Finish a login with the second factor.
#[utoipa::path(
    post,
    path = "/api/auth/login/mfa",
    tag = "auth",
    request_body = CodeInput,
    responses(
        (status = 204, description = "Logged in; `Set-Cookie: invoice_session=…`, the pending cookie cleared"),
        (status = 401, description = "`invalid_credentials` (no / expired / used pending login; a Google invitation accept whose invitation is gone), `mfa_invalid` (wrong code)", body = ErrorBody),
        (status = 429, description = "`rate_limited` (+ `Retry-After`)", body = ErrorBody),
    )
)]
pub async fn login_mfa(
    State(state): State<AppState>,
    Extension(host): Extension<HostCtx>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    ApiJson(input): ApiJson<CodeInput>,
) -> Result<Response, AppError> {
    let cookie =
        session::named_cookie(&headers, PENDING_COOKIE).ok_or(AppError::InvalidCredentials)?;
    let space = host.space_id();
    let v = Verifier::of(&state);
    // One transaction, the pending row locked first: parallel attempts on
    // one pending login run in turn (failure cap, single use), and a code is
    // spent only together with the session it buys.
    let outcome = state
        .db
        .transaction::<_, Outcome, AppError>(|txn| {
            Box::pin(async move {
                let pending = repo::lock_login(txn, &cookie, space)
                    .await?
                    .ok_or(AppError::InvalidCredentials)?;
                let invite = pending.invite_hash.clone();
                // Disabled, unverified or removed from the space meanwhile
                // (an invitation accept joins the space only now).
                let user = match users::find(txn, pending.user_id).await? {
                    Some(u) if invite.is_some() && !u.disabled => u,
                    Some(u) if invite.is_none() && may_sign_in(txn, &u, space).await? => u,
                    _ => return Err(AppError::InvalidCredentials),
                };
                let buckets = [(LOGIN_EMAIL, user.email.as_str()), (LOGIN_IP, ip.as_str())];
                let ok = match (invite, space) {
                    (Some(hash), Some(sp)) => {
                        // A savepoint: a wrong code rolls the used-up
                        // invitation back, but its failure is counted.
                        let inner = txn.begin().await?;
                        let code = Code {
                            verifier: &v,
                            code: &input.code,
                            buckets: &buckets,
                        };
                        match join(&inner, sp, &hash, Joiner::Existing(user.id), Some(code)).await?
                        {
                            Joined::Member(_) => {
                                inner.commit().await?;
                                true
                            }
                            Joined::WrongCode => {
                                inner.rollback().await?;
                                false
                            }
                            Joined::Refused => {
                                inner.commit().await?;
                                repo::take_login(txn, pending.id).await?;
                                return Ok(Outcome::Refused);
                            }
                        }
                    }
                    (Some(_), None) => return Err(AppError::InvalidCredentials),
                    (None, _) => verify::check(txn, &v, user.id, &input.code, &buckets).await?,
                };
                if !ok {
                    repo::fail_login(txn, &pending).await?;
                    return Ok(Outcome::WrongCode);
                }
                repo::take_login(txn, pending.id).await?;
                let method = Method::parse(&pending.login_method).unwrap_or(Method::Password);
                session::create(txn, user.id, space, pending.user_agent, method)
                    .await
                    .map(Outcome::Session)
            })
        })
        .await?;
    // A failure was committed (counted, or the invitation used up); now
    // answer it.
    let value = match outcome {
        Outcome::Session(v) => v,
        Outcome::WrongCode => return Err(AppError::MfaInvalid),
        Outcome::Refused => return Err(AppError::InvalidCredentials),
    };
    let https = state.public.https;
    with_cookies(
        StatusCode::NO_CONTENT.into_response(),
        &[&session::set_cookie(&value, https), &clear_pending(https)],
    )
}

enum Outcome {
    Session(String),
    WrongCode,
    /// A pending invitation accept whose invitation is gone or whose
    /// inviter may no longer grant it.
    Refused,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_cookie_attributes() {
        let c = pending_cookie("abc", true);
        assert!(c.starts_with("invoice_mfa=abc; HttpOnly; SameSite=Lax; Path=/;"));
        assert!(c.contains("Max-Age=300"));
        assert!(c.ends_with("; Secure"));
        assert!(!c.contains("Domain"));
        assert!(!pending_cookie("abc", false).contains("Secure"));
        assert!(clear_pending(false).starts_with("invoice_mfa=;"));
        assert!(clear_pending(false).contains("Max-Age=0"));
    }
}
