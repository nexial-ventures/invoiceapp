//! ID token verification against a JWKS: RS256 signature (the key by
//! `kid`), `iss`, `aud` = the client id, `exp`, `iat` not in the future
//! (60 s skew), `azp`, `nonce`, `email_verified`. Pure: the caller supplies the
//! key set (and refreshes it on [`Rejected::UnknownKid`]).

use jsonwebtoken::jwk::{Jwk, JwkSet, KeyAlgorithm};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use subtle::ConstantTimeEq;

/// Clock skew allowed for `exp` and `iat`.
pub const LEEWAY_SECS: u64 = 60;

/// What the token must match.
pub struct Expected<'a> {
    /// The discovery `issuer`.
    pub issuer: &'a str,
    pub client_id: &'a str,
    pub nonce: &'a str,
}

/// The verified identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub sub: String,
    /// Lowercased + trimmed.
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The header's `kid` is not in the key set (refresh it, then retry).
    UnknownKid,
    /// Google has not verified the e-mail address.
    UnverifiedEmail,
    /// Anything else; the reason is for the log only.
    Invalid(String),
}

/// Google sends `email_verified` as a boolean, some providers as a string.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Flag {
    Bool(bool),
    Text(String),
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    /// One value or a list (its match is checked by `Validation`).
    aud: serde_json::Value,
    azp: Option<String>,
    iat: i64,
    nonce: Option<String>,
    email: Option<String>,
    email_verified: Option<Flag>,
    name: Option<String>,
}

fn invalid(reason: impl Into<String>) -> Rejected {
    Rejected::Invalid(reason.into())
}

/// Google's ID tokens carry `iss` with or without the scheme.
fn issuers(issuer: &str) -> Vec<String> {
    let mut all = vec![issuer.to_string()];
    if let Some(bare) = issuer.strip_prefix("https://")
        && bare == "accounts.google.com"
    {
        all.push(bare.to_string());
    }
    all
}

fn key_for<'a>(keys: &'a JwkSet, kid: &str) -> Option<&'a Jwk> {
    keys.keys
        .iter()
        .find(|k| k.common.key_id.as_deref() == Some(kid))
}

/// Verify `token` at time `now` (unix seconds).
pub fn verify(
    token: &str,
    keys: &JwkSet,
    expected: &Expected<'_>,
    now: i64,
) -> Result<Identity, Rejected> {
    let header = decode_header(token).map_err(|e| invalid(format!("header: {e}")))?;
    if header.alg != Algorithm::RS256 {
        return Err(invalid(format!("algorithm {:?}", header.alg)));
    }
    let kid = header.kid.ok_or_else(|| invalid("no kid"))?;
    let jwk = key_for(keys, &kid).ok_or(Rejected::UnknownKid)?;
    if jwk
        .common
        .key_algorithm
        .is_some_and(|a| a != KeyAlgorithm::RS256)
    {
        return Err(invalid("key algorithm"));
    }
    let key = DecodingKey::from_jwk(jwk).map_err(|e| invalid(format!("key: {e}")))?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = LEEWAY_SECS;
    validation.set_issuer(&issuers(expected.issuer));
    validation.set_audience(&[expected.client_id]);
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub"]);
    let claims = decode::<Claims>(token, &key, &validation)
        .map_err(|e| invalid(format!("token: {e}")))?
        .claims;
    // OIDC Core 3.1.3.7: several audiences need `azp`; a present `azp` must
    // be us.
    let many = claims.aud.as_array().is_some_and(|a| a.len() > 1);
    match claims.azp.as_deref() {
        Some(azp) if azp != expected.client_id => return Err(invalid("azp")),
        None if many => return Err(invalid("several audiences without azp")),
        _ => {}
    }
    if claims.iat > now + LEEWAY_SECS as i64 {
        return Err(invalid("iat in the future"));
    }
    let nonce_ok = claims
        .nonce
        .as_deref()
        .is_some_and(|n| bool::from(n.as_bytes().ct_eq(expected.nonce.as_bytes())));
    if !nonce_ok {
        return Err(invalid("nonce"));
    }
    if claims.sub.is_empty() {
        return Err(invalid("empty sub"));
    }
    let email = claims
        .email
        .map(|e| crate::auth::users::normalize_email(&e))
        .filter(|e| !e.is_empty())
        .ok_or_else(|| invalid("no email"))?;
    let verified = match claims.email_verified {
        Some(Flag::Bool(b)) => b,
        Some(Flag::Text(t)) => t.eq_ignore_ascii_case("true"),
        None => false,
    };
    if !verified {
        return Err(Rejected::UnverifiedEmail);
    }
    Ok(Identity {
        sub: claims.sub,
        email,
        name: claims
            .name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty()),
    })
}

#[cfg(test)]
#[path = "idtoken_tests.rs"]
mod tests;
