use std::fmt;

use serde::Deserialize;

/// Wrapper that keeps sensitive values out of logs: `Debug` and `Display`
/// both print `[REDACTED]`. Use [`Secret::expose`] to read the inner value.
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// `INVOICEAPP__SECRET_KEY`: 32 bytes (base64 in the environment). Encrypts
/// TOTP secrets at rest and keys the recovery-code HMAC. `Debug` is
/// redacted; the `Default` all-zero value is only the placeholder of a
/// config that has not been parsed yet (`Config::from_source` always sets
/// it or fails).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretKey([u8; 32]);

impl SecretKey {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Standard base64 (padded or not) of exactly 32 bytes, e.g. the output
    /// of `openssl rand -base64 32`.
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        use base64::Engine as _;
        use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
        let raw = raw.trim();
        let bytes = STANDARD
            .decode(raw)
            .or_else(|_| STANDARD_NO_PAD.decode(raw))
            .map_err(|_| anyhow::anyhow!("INVOICEAPP__SECRET_KEY is not valid base64"))?;
        let key: [u8; 32] = bytes.try_into().map_err(|b: Vec<u8>| {
            anyhow::anyhow!(
                "INVOICEAPP__SECRET_KEY must be 32 bytes, got {} (generate one with `openssl rand -base64 32`)",
                b.len()
            )
        })?;
        Ok(Self(key))
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::{Secret, SecretKey};

    #[test]
    fn secret_key_parsing() {
        let b64 = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
        let key = SecretKey::parse(b64).expect("valid key");
        assert_eq!(key.bytes()[31], 31);
        assert!(SecretKey::parse(b64.trim_end_matches('=')).is_ok_and(|k| k == key));
        assert!(SecretKey::parse(&format!(" {b64}\n")).is_ok());
        assert!(SecretKey::parse("").is_err());
        assert!(SecretKey::parse("not base64!").is_err());
        // 31 and 33 bytes.
        assert!(SecretKey::parse("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHg==").is_err());
        assert!(SecretKey::parse("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8g").is_err());
        assert_eq!(format!("{key:?}"), "[REDACTED]");
    }

    #[test]
    fn debug_and_display_are_redacted() {
        let s = Secret::new("hunter2".to_string());
        assert_eq!(format!("{s:?}"), "[REDACTED]");
        assert_eq!(format!("{s}"), "[REDACTED]");
        assert_eq!(s.expose(), "hunter2");
    }

    #[test]
    fn redacted_inside_a_derived_debug() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            token: Secret<String>,
        }
        let h = Holder {
            token: Secret::new("hunter2".into()),
        };
        assert!(!format!("{h:?}").contains("hunter2"));
    }
}
