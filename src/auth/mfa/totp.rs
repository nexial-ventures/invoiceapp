//! RFC 6238 TOTP (HMAC-SHA1, 6 digits, 30 s step) and the RFC 4648 base32
//! of the secret. Pure: the caller supplies the time and the replay guard.

use hmac::{Hmac, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;

pub const STEP_SECS: i64 = 30;
pub const DIGITS: u32 = 6;
/// Secret length (bytes): 160 bits, the RFC 4226 recommendation.
pub const SECRET_LEN: usize = 20;
/// Steps accepted around the current one.
const WINDOW: i64 = 1;

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32, uppercase, no padding.
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buf >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buf << (5 - bits)) & 31) as usize]));
    }
    out
}

/// The 30 s step of a Unix time.
pub fn step_at(unix_secs: i64) -> i64 {
    unix_secs.div_euclid(STEP_SECS)
}

/// HOTP (RFC 4226) of `counter` with `digits` digits.
pub fn hotp(secret: &[u8], counter: u64, digits: u32) -> u32 {
    let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(secret) else {
        // HMAC accepts keys of any length; unreachable in practice.
        return u32::MAX;
    };
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let offset = usize::from(h[19] & 0x0f);
    let bin = u32::from_be_bytes([
        h[offset] & 0x7f,
        h[offset + 1],
        h[offset + 2],
        h[offset + 3],
    ]);
    bin % 10u32.pow(digits)
}

/// The 6-digit code of `step`, zero-padded.
pub fn code(secret: &[u8], step: i64) -> String {
    let counter = u64::try_from(step).unwrap_or_default();
    format!("{:06}", hotp(secret, counter, DIGITS))
}

/// Is `input` (already normalized: 6 ASCII digits) the code of a step
/// within ±1 of `now_step` and later than `last_step`? Returns the matched
/// step (to be stored as the new `last_step`). Every candidate is compared
/// in constant time.
pub fn verify(secret: &[u8], input: &str, now_step: i64, last_step: Option<i64>) -> Option<i64> {
    if input.len() != DIGITS as usize || !input.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut found = None;
    for step in now_step - WINDOW..=now_step + WINDOW {
        let matches: bool = code(secret, step).as_bytes().ct_eq(input.as_bytes()).into();
        let fresh = last_step.is_none_or(|last| step > last);
        if matches && fresh && found.is_none() {
            found = Some(step);
        }
    }
    found
}

/// Percent-encode everything but unreserved characters and `@ ( )`.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'@'
            | b'('
            | b')' => char::from(b).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `otpauth://totp/InvoiceApp%20({host}):{email}?secret=…&issuer=InvoiceApp%20({host})&digits=6&period=30`.
pub fn otpauth_uri(base_host: &str, email: &str, secret_b32: &str) -> String {
    let issuer = encode(&format!("InvoiceApp ({base_host})"));
    format!(
        "otpauth://totp/{issuer}:{}?secret={secret_b32}&issuer={issuer}&digits={DIGITS}&period={STEP_SECS}",
        encode(email)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B, SHA-1 (8 digits, the ASCII seed "1234567890" × 2).
    const SEED: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc6238_sha1_vectors() {
        let vectors = [
            (59, 94_287_082),
            (1_111_111_109, 7_081_804),
            (1_111_111_111, 14_050_471),
            (1_234_567_890, 89_005_924),
            (2_000_000_000, 69_279_037),
            (20_000_000_000, 65_353_130),
        ];
        for (t, expected) in vectors {
            let step = u64::try_from(step_at(t)).expect("positive");
            assert_eq!(hotp(SEED, step, 8), expected, "T = {t}");
            // The 6-digit code is the low 6 digits of the same value.
            assert_eq!(
                code(SEED, step_at(t)),
                format!("{:06}", expected % 1_000_000)
            );
        }
    }

    #[test]
    fn window_is_plus_minus_one_step() {
        let now = step_at(1_700_000_000);
        for delta in -1..=1 {
            let c = code(SEED, now + delta);
            assert_eq!(verify(SEED, &c, now, None), Some(now + delta), "{delta}");
        }
        for delta in [-2, 2] {
            let c = code(SEED, now + delta);
            assert_eq!(verify(SEED, &c, now, None), None, "{delta}");
        }
    }

    #[test]
    fn replay_is_refused() {
        let now = step_at(1_700_000_000);
        let c = code(SEED, now);
        let step = verify(SEED, &c, now, None).expect("first use");
        assert_eq!(verify(SEED, &c, now, Some(step)), None);
        // An older code after a newer one was used.
        let prev = code(SEED, now - 1);
        assert_eq!(verify(SEED, &prev, now, Some(step)), None);
        // The next step is fine.
        let next = code(SEED, now + 1);
        assert_eq!(verify(SEED, &next, now, Some(step)), Some(now + 1));
    }

    #[test]
    fn malformed_codes_never_match() {
        let now = step_at(1_700_000_000);
        for bad in ["", "12345", "1234567", "abcdef", " 12345"] {
            assert_eq!(verify(SEED, bad, now, None), None, "{bad:?}");
        }
    }

    #[test]
    fn base32_rfc4648() {
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "MY");
        assert_eq!(base32(b"fo"), "MZXQ");
        assert_eq!(base32(b"foo"), "MZXW6");
        assert_eq!(base32(b"foob"), "MZXW6YQ");
        assert_eq!(base32(b"fooba"), "MZXW6YTB");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32(SEED), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    }

    #[test]
    fn otpauth_uri_format() {
        assert_eq!(
            otpauth_uri("invoiceapp.cz", "jan+a@example.com", "ABC"),
            "otpauth://totp/InvoiceApp%20(invoiceapp.cz):jan%2Ba@example.com?secret=ABC\
             &issuer=InvoiceApp%20(invoiceapp.cz)&digits=6&period=30"
        );
    }
}
