//! `GET /api/auth/google/finish?token` on the origin host: consumes the
//! handoff, checks the browser binding, and only then resolves the user
//! (auto-link, claim, new account) and completes the purpose — a session
//! (or the TOTP code step), an invitation accept, or linking the account.
//! Every failure, internal ones included (logged), is an error redirect.

use axum::Extension;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sea_orm::TransactionTrait;

use super::entity::google_handoff;
use super::flow::{self, Purpose};
use super::{Fail, account, enabled, error_location, redirect};
use crate::app::AppState;
use crate::auth::handlers::login::{may_sign_in, second_factor};
use crate::auth::host::HostCtx;
use crate::auth::mfa::{self, repo::Pending};
use crate::auth::session::{self, Method};
use crate::auth::{crypto, ctx, users};
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiQuery;
use crate::members::join::{Joined, Joiner, join};

pub use crate::members::accept::TokenQuery;

/// Where a TOTP user continues: the SPA's code step.
const MFA_STEP: &str = "/login?mfa=1";

/// The end of a purpose: a redirect path on this host and the cookies.
struct Done {
    path: String,
    cookies: Vec<String>,
}

/// Complete the sign-in on the host it started on.
#[utoipa::path(
    get,
    path = "/api/auth/google/finish",
    tag = "auth",
    params(("token" = String, Query, description = "The handoff token (60 s, single use)")),
    responses(
        (status = 302, description = "Signed in → `return` (`Set-Cookie: invoice_session=…`); TOTP user → `/login?mfa=1` (`Set-Cookie: invoice_mfa=…`); linked → `/account?google=linked`; else an error redirect with `google_error` = `expired | invalid | disabled | not_member | mfa_required | already_linked | email_mismatch | unavailable`"),
        (status = 404, description = "Not configured", body = ErrorBody),
    )
)]
pub async fn finish(
    State(state): State<AppState>,
    Extension(host): Extension<HostCtx>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<TokenQuery>,
) -> Result<Response, AppError> {
    enabled(&state)?;
    let origin = super::host_url(&state, &host);
    let h = match flow::take_handoff(&state.db, q.token.trim(), host.space_id()).await {
        Ok(Some(h)) => Ok(h),
        Ok(None) => Err(Fail::Invalid),
        Err(e) => Err(Fail::from(e)),
    };
    let h = match h {
        Ok(h) => h,
        Err(f) => return redirect(&error_location(&origin, Purpose::Login, None, f), &[]),
    };
    // Only this flow's binding cookie; parallel flows keep theirs.
    let clear = session::cookie(
        &h.browser_cookie,
        "",
        chrono::Duration::zero(),
        state.public.https,
    );
    let purpose = Purpose::parse(&h.purpose).unwrap_or(Purpose::Login);
    let invite = flow::open_invite(&state.secret_key, h.space_id, h.invite_sealed.as_deref());
    let browser_ok = session::named_cookie(&headers, &h.browser_cookie)
        .is_some_and(|c| crypto::digest(&c) == h.browser_hash);
    let done = if flow::expired(h.expires_at) {
        Err(Fail::Expired)
    } else if !browser_ok {
        Err(Fail::Invalid)
    } else {
        match purpose {
            Purpose::Login => login(&state, &host, &headers, &h).await,
            Purpose::Invite => accept(&state, &host, &headers, &h).await,
            Purpose::Link => link(&state, &host, &headers, &h).await,
        }
    };
    match done {
        Ok(d) => {
            let mut cookies: Vec<&str> = d.cookies.iter().map(String::as_str).collect();
            cookies.push(&clear);
            redirect(&format!("{origin}{}", d.path), &cookies)
        }
        Err(f) => redirect(
            &error_location(&origin, purpose, invite.as_deref(), f),
            &[&clear],
        ),
    }
}

/// The TOTP rule of a password login: the code step, `mfa_required`, or
/// `None` = sign in now.
async fn totp_step(
    state: &AppState,
    host: &HostCtx,
    user_id: uuid::Uuid,
    ua: Option<String>,
    pending: Pending,
) -> Result<Option<Done>, Fail> {
    match second_factor(state, host, user_id, ua, pending).await {
        Ok(Some(value)) => Ok(Some(Done {
            path: MFA_STEP.into(),
            cookies: vec![mfa::pending_cookie(&value, state.public.https)],
        })),
        Ok(None) => Ok(None),
        Err(AppError::MfaRequired { .. }) => Err(Fail::MfaRequired),
        Err(e) => Err(e.into()),
    }
}

async fn login(
    state: &AppState,
    host: &HostCtx,
    headers: &HeaderMap,
    h: &google_handoff::Model,
) -> Result<Done, Fail> {
    let space = host.space_id();
    let base_host = space.is_none();
    let create = account::may_create(false, base_host, state.registration);
    let uid = match account::resolve(&state.db, &flow::identity(h), create).await {
        // No account behind a space host's login: not a member.
        Err(Fail::NotAllowed) if !base_host => return Err(Fail::NotMember),
        r => r?,
    };
    let user = users::find(&state.db, uid).await?.ok_or(Fail::Invalid)?;
    if user.disabled {
        return Err(Fail::Disabled);
    }
    if !may_sign_in(&state.db, &user, space).await? {
        return Err(Fail::NotMember);
    }
    let ua = session::user_agent(headers);
    let pending = Pending::login(Method::Google);
    if let Some(step) = totp_step(state, host, user.id, ua.clone(), pending).await? {
        return Ok(step);
    }
    let value = session::create(&state.db, user.id, space, ua, Method::Google).await?;
    Ok(Done {
        path: h.return_path.clone(),
        cookies: vec![session::set_cookie(&value, state.public.https)],
    })
}

/// The 4b / 4c accept rules with Google as the first factor.
async fn accept(
    state: &AppState,
    host: &HostCtx,
    headers: &HeaderMap,
    h: &google_handoff::Model,
) -> Result<Done, Fail> {
    let space = host.space_id().ok_or(Fail::Invalid)?;
    let hash = h.invite_hash.clone().ok_or(Fail::Invalid)?;
    let id = flow::identity(h);
    // Before any account is created or linked: still live, for this address.
    let invite = super::handlers::invitation(state, h.space_id, Some(&hash), &id.email).await?;
    let uid = account::resolve(&state.db, &id, account::may_create(true, false, false)).await?;
    let user = users::find(&state.db, uid).await?.ok_or(Fail::Invalid)?;
    if user.disabled {
        return Err(Fail::Disabled);
    }
    // Found by `sub`: the account itself must be the invited one.
    if invite.email != user.email {
        return Err(Fail::EmailMismatch);
    }
    let ua = session::user_agent(headers);
    // With TOTP the membership is created only after the code (login/mfa).
    let pending = Pending {
        method: Method::Google,
        invite_hash: Some(hash.clone()),
    };
    if let Some(step) = totp_step(state, host, user.id, ua.clone(), pending).await? {
        return Ok(step);
    }
    let uid = user.id;
    let value = state
        .db
        .transaction::<_, Option<String>, AppError>(|txn| {
            Box::pin(async move {
                match join(txn, space, &hash, Joiner::Existing(uid), None).await? {
                    Joined::Member(id) => session::create(txn, id, Some(space), ua, Method::Google)
                        .await
                        .map(Some),
                    // Committed: a failed inviter re-check keeps the link dead.
                    Joined::Refused | Joined::WrongCode => Ok(None),
                }
            })
        })
        .await
        .map_err(|e| Fail::from(AppError::from(e)))?
        .ok_or(Fail::Invalid)?;
    Ok(Done {
        path: h.return_path.clone(),
        cookies: vec![session::set_cookie(&value, state.public.https)],
    })
}

/// Attach the Google account to the session's user (no new login).
async fn link(
    state: &AppState,
    host: &HostCtx,
    headers: &HeaderMap,
    h: &google_handoff::Model,
) -> Result<Done, Fail> {
    let sub = h.google_sub.as_str();
    let owner = h.user_id.ok_or(Fail::Invalid)?;
    let cookie = session::cookie_value(headers).ok_or(Fail::Invalid)?;
    let authed = match ctx::by_session(state, host, &cookie).await {
        Ok(a) => a,
        Err(AppError::Unauthorized) => return Err(Fail::Invalid),
        Err(e) => return Err(e.into()),
    };
    if authed.user.id != owner {
        return Err(Fail::Invalid);
    }
    account::check_link(&state.db, owner, sub).await?;
    if !account::attach(&state.db, owner, sub).await? {
        return Err(Fail::AlreadyLinked);
    }
    Ok(Done {
        path: "/account?google=linked".into(),
        cookies: Vec::new(),
    })
}
