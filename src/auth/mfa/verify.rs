//! Checking a second-factor code: a TOTP code (±1 step, replay-guarded) or
//! a single-use recovery code, wherever a code is accepted (login step,
//! step-up confirmations). Failures count in the code bucket (`MFA_EMAIL`, hard).
//!
//! [`check`] takes any connection: callers whose operation can still fail
//! run it inside that operation's transaction, after its own checks, so a
//! failed request rolls the spent code back.

use chrono::Utc;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

use super::{repo, seal, totp};
use crate::app::AppState;
use crate::auth::entity::user;
use crate::auth::ratelimit::{Limit, MFA_EMAIL, RateLimiter};
use crate::error::{AppError, FieldErrors};
use crate::secret::SecretKey;

/// What a code check needs besides the connection.
#[derive(Clone)]
pub struct Verifier {
    pub limiter: RateLimiter,
    pub key: SecretKey,
}

impl Verifier {
    pub fn of(state: &AppState) -> Self {
        Self {
            limiter: state.limiter.clone(),
            key: state.secret_key.clone(),
        }
    }
}

fn is_totp_shape(code: &str) -> bool {
    code.len() == totp::DIGITS as usize && code.bytes().all(|b| b.is_ascii_digit())
}

/// Spend `raw` as the user's code: `true` for a fresh TOTP code (its step is
/// recorded) or an unused recovery code (deleted). One attempt is reserved
/// in `buckets` first (429 when used up) and given back on success. A user
/// without TOTP has no valid code.
///
/// A TOTP secret that does not decrypt (`INVOICE__SECRET_KEY` changed) is
/// logged and answered as a wrong code **without** a reservation, so
/// recovery codes — which need no decryption — stay usable.
pub async fn check(
    db: &impl ConnectionTrait,
    v: &Verifier,
    user_id: Uuid,
    raw: &str,
    buckets: &[(Limit, &str)],
) -> Result<bool, AppError> {
    let row = repo::totp(db, user_id).await?;
    let Some(sealed) = row.totp_secret.as_deref() else {
        return Ok(false);
    };
    let code = seal::normalize_code(raw);
    if is_totp_shape(&code) {
        let Ok(secret) = seal::open(&v.key, user_id, sealed) else {
            tracing::error!(
                user = %user_id,
                "TOTP secret cannot be decrypted (INVOICE__SECRET_KEY changed?); only recovery codes work"
            );
            return Ok(false);
        };
        v.limiter.reserve(buckets)?;
        let now = totp::step_at(Utc::now().timestamp());
        let ok = match totp::verify(&secret, &code, now, row.totp_last_step) {
            // The conditional update is the real replay guard (a parallel
            // request with the same code loses here).
            Some(step) => repo::advance_step(db, user_id, step).await?,
            None => false,
        };
        if ok {
            v.limiter.refund(buckets);
        }
        return Ok(ok);
    }
    v.limiter.reserve(buckets)?;
    let ok = seal::looks_like_recovery(&code)
        && repo::use_recovery(db, user_id, seal::recovery_hash(&v.key, &code)).await?;
    if ok {
        v.limiter.refund(buckets);
    }
    Ok(ok)
}

/// A setup's first code against the pending (not yet active) secret, in the
/// user's code bucket. Returns the matched step.
pub async fn check_pending(
    state: &AppState,
    user: &user::Model,
    sealed: &[u8],
    raw: &str,
) -> Result<Option<i64>, AppError> {
    let secret = seal::open(&state.secret_key, user.id, sealed)?;
    let bucket = [(MFA_EMAIL, user.email.as_str())];
    state.limiter.reserve(&bucket)?;
    let now = totp::step_at(Utc::now().timestamp());
    let step = totp::verify(&secret, &seal::normalize_code(raw), now, None);
    if step.is_some() {
        state.limiter.refund(&bucket);
    }
    Ok(step)
}

/// The step-up rule's first half: for a user with TOTP a missing `code` →
/// `code: required` (reported with the other field errors). Returns the code
/// to check — only when nothing else failed, so a request that fails anyway
/// never spends one. Users without TOTP: nothing is asked, a sent code is
/// ignored.
pub fn pending_code<'a>(
    mfa_enabled: bool,
    code: Option<&'a str>,
    e: &mut FieldErrors,
) -> Option<&'a str> {
    if !mfa_enabled {
        return None;
    }
    let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) else {
        e.add("code", "required");
        return None;
    };
    e.is_empty().then_some(code)
}

/// The full step-up rule on the pool (routes whose work after the check
/// cannot fail for a client reason): [`pending_code`], then [`check`] in the
/// user's code bucket; wrong → `code: invalid`.
pub async fn step_up(
    state: &AppState,
    user: &user::Model,
    mfa_enabled: bool,
    code: Option<&str>,
    e: &mut FieldErrors,
) -> Result<(), AppError> {
    let Some(code) = pending_code(mfa_enabled, code, e) else {
        return Ok(());
    };
    let bucket = [(MFA_EMAIL, user.email.as_str())];
    if !check(&state.db, &Verifier::of(state), user.id, code, &bucket).await? {
        e.add("code", "invalid");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_up_code_rule() {
        let mut e = FieldErrors::new();
        assert_eq!(pending_code(false, Some("x"), &mut e), None);
        assert!(e.is_empty());
        assert_eq!(pending_code(true, Some(" 123456 "), &mut e), Some("123456"));
        assert_eq!(pending_code(true, Some("  "), &mut e), None);
        assert_eq!(e.get("code"), Some("required"));
        let mut e = FieldErrors::new();
        e.add("password", "invalid");
        assert_eq!(pending_code(true, Some("123456"), &mut e), None);
        assert_eq!(e.get("code"), None);
    }
}
