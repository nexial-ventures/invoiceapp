//! Space slug rules: `[a-z0-9-]`, 3–30 chars, no leading / trailing `-`,
//! not a reserved name. Slugs are immutable once created.

/// Infrastructure names, and names a phishing page would pick
/// (`docs/api/spaces.md`). Shorter than 3 characters is already invalid.
const RESERVED: [&str; 52] = [
    "www",
    "api",
    "app",
    "admin",
    "mail",
    "smtp",
    "static",
    "assets",
    "cdn",
    "status",
    "docs",
    "help",
    "support",
    "blog",
    "login",
    "signin",
    "signup",
    "register",
    "account",
    "accounts",
    "auth",
    "oauth",
    "sso",
    "secure",
    "security",
    "verify",
    "verification",
    "password",
    "reset",
    "billing",
    "pay",
    "payment",
    "payments",
    "paypal",
    "bank",
    "banking",
    "google",
    "microsoft",
    "apple",
    "root",
    "system",
    "email",
    "webmail",
    "imap",
    "ns",
    "dns",
    "invoice",
    "invoices",
    "faktura",
    "faktury",
    "test",
    "demo",
];

/// The slug as sent (trimmed only) or the 422 reason: `required`,
/// `invalid`, `reserved`. Upper-case letters are invalid, not folded, so
/// the URL the user typed is the URL they get.
pub fn validate(raw: &str) -> Result<String, &'static str> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("required");
    }
    if !shape_ok(s) {
        return Err("invalid");
    }
    if RESERVED.contains(&s) {
        return Err("reserved");
    }
    Ok(s.to_string())
}

fn shape_ok(s: &str) -> bool {
    (3..=30).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// Could `label` (one DNS label of a request host) be a space slug? Used by
/// the host classification before any DB lookup. Shape only: a name
/// reserved after a space took it must keep resolving.
pub fn plausible(label: &str) -> bool {
    shape_ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_rules() {
        assert_eq!(validate(" firma ").as_deref(), Ok("firma"));
        assert_eq!(validate("a-1").as_deref(), Ok("a-1"));
        assert_eq!(validate(&"a".repeat(30)).map(|s| s.len()), Ok(30));
        assert_eq!(validate(""), Err("required"));
        assert_eq!(validate("  "), Err("required"));
        assert_eq!(validate("ab"), Err("invalid"));
        assert_eq!(validate(&"a".repeat(31)), Err("invalid"));
        assert_eq!(validate("-abc"), Err("invalid"));
        assert_eq!(validate("abc-"), Err("invalid"));
        assert_eq!(validate("Firma"), Err("invalid"));
        assert_eq!(validate("fir.ma"), Err("invalid"));
        assert_eq!(validate("fír"), Err("invalid"));
        assert_eq!(validate("www"), Err("reserved"));
        assert_eq!(validate("support"), Err("reserved"));
        for name in [
            "login", "paypal", "google", "faktura", "invoice", "test", "demo", "webmail",
        ] {
            assert_eq!(validate(name), Err("reserved"), "{name}");
        }
        assert_eq!(validate("ns"), Err("invalid"), "too short before reserved");
        assert_eq!(validate("login-firma").as_deref(), Ok("login-firma"));
        assert!(plausible("firma"));
        assert!(
            plausible("api"),
            "reserved names are refused only at creation"
        );
        assert!(!plausible("ab") && !plausible("-ab") && !plausible("Firma"));
    }
}
