//! What `INVOICEAPP__SECRET_KEY` protects: TOTP secrets sealed with
//! AES-256-GCM (random 96-bit nonce per value, the user id as associated
//! data so a sealed secret cannot be moved to another user) and the
//! HMAC-SHA256 of recovery codes. Two subkeys are derived from the master
//! key (HMAC-SHA256 over a fixed label), so neither use weakens the other.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::anyhow;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

use crate::error::AppError;
use crate::secret::SecretKey;

const NONCE_LEN: usize = 12;
/// Recovery codes per user.
pub const RECOVERY_CODES: usize = 10;
/// Base32 characters of a recovery code (50 bits).
const RECOVERY_CHARS: usize = 10;
const LOWER_B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn hmac256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let Ok(mut mac) = <Hmac<Sha256> as Mac>::new_from_slice(key) else {
        // HMAC accepts keys of any length; unreachable in practice.
        return [0; 32];
    };
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn subkey(key: &SecretKey, label: &[u8]) -> [u8; 32] {
    hmac256(key.bytes(), label)
}

fn cipher(key: &SecretKey) -> Result<Aes256Gcm, AppError> {
    Aes256Gcm::new_from_slice(&subkey(key, b"invoice/totp-secret/aes-256-gcm"))
        .map_err(|_| anyhow!("AES key setup failed").into())
}

/// `nonce || ciphertext || tag` of `plain`, bound to `user`.
pub fn seal(key: &SecretKey, user: Uuid, plain: &[u8]) -> Result<Vec<u8>, AppError> {
    let nonce = crate::auth::crypto::random_bytes::<NONCE_LEN>()?;
    let sealed = cipher(key)?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: user.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("TOTP secret encryption failed"))?;
    Ok([nonce.as_slice(), &sealed].concat())
}

/// The plaintext of [`seal`]'s output; a wrong key, user or a tampered
/// value → an internal error (logged, never shown).
pub fn open(key: &SecretKey, user: Uuid, sealed: &[u8]) -> Result<Vec<u8>, AppError> {
    if sealed.len() <= NONCE_LEN {
        return Err(anyhow!("sealed TOTP secret too short").into());
    }
    let (nonce, body) = sealed.split_at(NONCE_LEN);
    cipher(key)?
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: body,
                aad: user.as_bytes(),
            },
        )
        .map_err(|_| {
            anyhow!("TOTP secret cannot be decrypted (INVOICEAPP__SECRET_KEY changed?)").into()
        })
}

/// Lowercase, without `-` and whitespace.
pub fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Could the normalized input be a recovery code (10 base32 characters)?
pub fn looks_like_recovery(normalized: &str) -> bool {
    normalized.len() == RECOVERY_CHARS && normalized.bytes().all(|b| LOWER_B32.contains(&b))
}

/// What the database stores of a recovery code: HMAC-SHA256 (hex) of the
/// normalized code.
pub fn recovery_hash(key: &SecretKey, normalized: &str) -> String {
    let mac = hmac256(
        &subkey(key, b"invoice/recovery-code/hmac-sha256"),
        normalized.as_bytes(),
    );
    mac.iter().map(|b| format!("{b:02x}")).collect()
}

/// One code `xxxxx-xxxxx`: 50 random bits as lowercase base32.
fn recovery_code() -> Result<String, AppError> {
    let bytes = crate::auth::crypto::random_bytes::<8>()?;
    let mut bits = u64::from_be_bytes(bytes) >> 14;
    let mut chars = [0u8; RECOVERY_CHARS];
    for c in chars.iter_mut().rev() {
        *c = LOWER_B32[(bits & 31) as usize];
        bits >>= 5;
    }
    let s: String = chars.iter().map(|b| char::from(*b)).collect();
    Ok(format!("{}-{}", &s[..5], &s[5..]))
}

/// [`RECOVERY_CODES`] fresh, distinct codes.
pub fn recovery_codes() -> Result<Vec<String>, AppError> {
    let mut out: Vec<String> = Vec::with_capacity(RECOVERY_CODES);
    while out.len() < RECOVERY_CODES {
        let c = recovery_code()?;
        if !out.contains(&c) {
            out.push(c);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> SecretKey {
        SecretKey::from_bytes([b; 32])
    }

    #[test]
    fn aes_gcm_round_trip_and_tamper_detection() {
        let user = Uuid::new_v4();
        let sealed = seal(&key(1), user, b"totp secret 20 bytes").expect("seal");
        assert_eq!(
            open(&key(1), user, &sealed).expect("open"),
            b"totp secret 20 bytes"
        );
        // A fresh nonce each time.
        assert_ne!(
            sealed,
            seal(&key(1), user, b"totp secret 20 bytes").expect("seal")
        );
        // Wrong key, another user, a flipped bit, a truncated value.
        assert!(open(&key(2), user, &sealed).is_err());
        assert!(open(&key(1), Uuid::new_v4(), &sealed).is_err());
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(open(&key(1), user, &tampered).is_err());
        assert!(open(&key(1), user, &sealed[..NONCE_LEN]).is_err());
    }

    #[test]
    fn recovery_codes_shape() {
        let codes = recovery_codes().expect("codes");
        assert_eq!(codes.len(), RECOVERY_CODES);
        for c in &codes {
            assert_eq!(c.len(), 11, "{c}");
            assert_eq!(&c[5..6], "-");
            assert!(looks_like_recovery(&normalize_code(c)), "{c}");
        }
    }

    #[test]
    fn recovery_normalisation_and_hmac() {
        assert_eq!(normalize_code(" AbCdE-fGh2 3 "), "abcdefgh23");
        assert!(looks_like_recovery("abcdefgh23"));
        assert!(!looks_like_recovery("abcdefgh2"));
        assert!(!looks_like_recovery("abcdefgh18"));
        let h = recovery_hash(&key(1), "abcdefgh23");
        assert_eq!(h.len(), 64);
        assert_eq!(h, recovery_hash(&key(1), &normalize_code("ABCDE-FGH23")));
        assert_ne!(h, recovery_hash(&key(2), "abcdefgh23"));
        assert_ne!(h, recovery_hash(&key(1), "abcdefgh24"));
    }
}
