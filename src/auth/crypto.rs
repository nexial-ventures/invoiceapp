//! Secrets: random tokens, their sha256 digests, argon2id passwords, the
//! personal API token format. Nothing here ever logs a secret.

use std::num::NonZeroUsize;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use tokio::sync::Semaphore;

use crate::error::AppError;

pub const API_TOKEN_PREFIX: &str = "inv_";

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], AppError> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("OS random source failed: {e}"))?;
    Ok(buf)
}

/// 32 random bytes, base64url without padding (session cookies, verify /
/// reset tokens).
pub fn random_token() -> Result<String, AppError> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?))
}

/// What the database stores of a token: sha256, lowercase hex.
pub fn digest(token: &str) -> String {
    crate::storage::sha256_hex(token.as_bytes())
}

/// A new personal API token `inv_{prefix}_{secret}`: `prefix` 8 hex chars
/// (shown in lists), `secret` 32 random bytes base64url. Returns
/// `(token, prefix)`.
pub fn new_api_token() -> Result<(String, String), AppError> {
    let prefix: String = random_bytes::<4>()?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let secret = URL_SAFE_NO_PAD.encode(random_bytes::<32>()?);
    Ok((format!("{API_TOKEN_PREFIX}{prefix}_{secret}"), prefix))
}

/// Does `token` have the API token shape? Anything else is refused before
/// a database lookup.
pub fn looks_like_api_token(token: &str) -> bool {
    token
        .strip_prefix(API_TOKEN_PREFIX)
        .and_then(|rest| rest.split_once('_'))
        .is_some_and(|(prefix, secret)| {
            prefix.len() == 8 && prefix.bytes().all(|b| b.is_ascii_hexdigit()) && !secret.is_empty()
        })
}

/// Extract the token from an `Authorization: Bearer <token>` header value.
/// The scheme is matched case-insensitively (RFC 7235).
pub fn parse_bearer(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// argon2id with the crate defaults (OWASP parameters), random salt.
fn hash_blocking(password: &str) -> Result<String, AppError> {
    let salt = SaltString::encode_b64(&random_bytes::<16>()?)
        .map_err(|e| anyhow!("encode password salt: {e}"))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("hash password: {e}"))?
        .to_string())
}

fn verify_blocking(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

/// A hash no password matches, verified for unknown e-mails so a login
/// takes the same time whether the account exists or not.
static DUMMY_HASH: LazyLock<Option<String>> =
    LazyLock::new(|| hash_blocking("dummy password that nobody has").ok());

/// A cap on concurrent argon2 runs (each costs ~19 MiB and a core): a burst
/// of logins must not exhaust memory or starve the blocking pool. A caller
/// that cannot get a permit within `wait` gets 503 `busy`.
struct Gate {
    permits: Arc<Semaphore>,
    wait: Duration,
}

impl Gate {
    fn new(permits: usize, wait: Duration) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(permits)),
            wait,
        }
    }

    async fn run<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let permit = tokio::time::timeout(self.wait, self.permits.clone().acquire_owned())
            .await
            .map_err(|_| AppError::Busy)?
            .map_err(|_| AppError::Busy)?;
        // The permit moves into the task: a cancelled request does not free
        // it while argon2 is still running.
        Ok(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            f()
        })
        .await
        .context("password hashing task")?)
    }
}

/// Two argon2 runs per core, at least two.
fn gate_permits(parallelism: Option<NonZeroUsize>) -> usize {
    parallelism
        .map_or(1, NonZeroUsize::get)
        .saturating_mul(2)
        .max(2)
}

static GATE: LazyLock<Gate> = LazyLock::new(|| {
    Gate::new(
        gate_permits(std::thread::available_parallelism().ok()),
        Duration::from_secs(2),
    )
});

/// 503 `busy` when the argon2 gate stays full for 2 s.
pub async fn hash_password(password: String) -> Result<String, AppError> {
    GATE.run(move || hash_blocking(&password)).await?
}

/// Verify `password` against `hash`; `None` runs the dummy verification and
/// returns `false`. 503 `busy` when the argon2 gate stays full for 2 s.
pub async fn verify_password(password: String, hash: Option<String>) -> Result<bool, AppError> {
    GATE.run(move || match hash {
        Some(h) => verify_blocking(&password, &h),
        None => {
            if let Some(dummy) = DUMMY_HASH.as_deref() {
                verify_blocking(&password, dummy);
            }
            false
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_url_safe() {
        let a = random_token().expect("token");
        let b = random_token().expect("token");
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_eq!(digest("abc").len(), 64);
        assert_eq!(digest("abc"), digest("abc"));
    }

    #[test]
    fn api_token_shape() {
        let (token, prefix) = new_api_token().expect("token");
        assert!(token.starts_with(&format!("inv_{prefix}_")));
        assert_eq!(prefix.len(), 8);
        assert!(looks_like_api_token(&token));
        assert!(!looks_like_api_token("inv_short_x"));
        assert!(!looks_like_api_token("inv_abcdefgh_"));
        assert!(!looks_like_api_token("tok_abcdef12_secret"));
        assert!(!looks_like_api_token("inv_zzzzzzzz_secret"));
    }

    #[test]
    fn parses_bearer_header() {
        assert_eq!(parse_bearer("Bearer tok"), Some("tok"));
        assert_eq!(parse_bearer("bearer tok"), Some("tok"));
        assert_eq!(parse_bearer("Bearer  tok "), Some("tok"));
        assert_eq!(parse_bearer("Basic tok"), None);
        assert_eq!(parse_bearer("Bearer "), None);
        assert_eq!(parse_bearer("tok"), None);
    }

    #[test]
    fn gate_permits_scale_with_cores() {
        assert_eq!(gate_permits(None), 2);
        assert_eq!(gate_permits(NonZeroUsize::new(1)), 2);
        assert_eq!(gate_permits(NonZeroUsize::new(8)), 16);
    }

    #[tokio::test]
    async fn a_full_gate_answers_busy() {
        let gate = Arc::new(Gate::new(1, Duration::from_millis(50)));
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let held = {
            let gate = gate.clone();
            tokio::spawn(async move { gate.run(move || rx.recv().is_ok()).await })
        };
        while gate.permits.available_permits() > 0 {
            tokio::task::yield_now().await;
        }
        assert!(matches!(gate.run(|| 1).await, Err(AppError::Busy)));
        tx.send(()).expect("release");
        assert!(held.await.expect("join").expect("ran"));
        assert_eq!(gate.run(|| 7).await.expect("free again"), 7);
    }

    #[tokio::test]
    async fn passwords_hash_and_verify() {
        let hash = hash_password("correct horse battery".into())
            .await
            .expect("hash");
        assert!(hash.starts_with("$argon2id$"));
        assert!(
            verify_password("correct horse battery".into(), Some(hash.clone()))
                .await
                .expect("verify")
        );
        assert!(
            !verify_password("wrong horse battery".into(), Some(hash))
                .await
                .expect("verify")
        );
        assert!(
            !verify_password("anything".into(), None)
                .await
                .expect("verify")
        );
    }
}
