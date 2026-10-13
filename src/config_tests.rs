use super::*;

const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// `pairs` plus a valid `INVOICEAPP__SECRET_KEY` unless `pairs` sets one.
pub(super) fn env(pairs: &[(&str, &str)]) -> Option<HashMap<String, String>> {
    let mut map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    map.entry("INVOICEAPP__SECRET_KEY".into())
        .or_insert_with(|| KEY.into());
    Some(map)
}

#[test]
fn secret_key_is_required_and_checked() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
    ];
    let cfg = Config::from_source(env(&base)).expect("valid config");
    assert_eq!(cfg.secret_key.bytes()[1], 1);
    assert!(!format!("{cfg:?}").contains(KEY));
    for bad in [
        "",
        " ",
        "short",
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHg==",
    ] {
        let mut pairs = base.to_vec();
        pairs.push(("INVOICEAPP__SECRET_KEY", bad));
        let err = Config::from_source(env(&pairs)).expect_err(bad);
        assert!(
            format!("{err:#}").contains("INVOICEAPP__SECRET_KEY"),
            "{err:#}"
        );
    }
    let mut map = env(&base).unwrap_or_default();
    map.remove("INVOICEAPP__SECRET_KEY");
    let err = Config::from_source(Some(map)).expect_err("missing key");
    assert!(
        format!("{err:#}").contains("INVOICEAPP__SECRET_KEY"),
        "{err:#}"
    );
}

#[test]
fn loads_with_default_bind() {
    let cfg = Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
    ]))
    .expect("valid config");
    assert_eq!(cfg.bind, DEFAULT_BIND);
    assert_eq!(cfg.ares_url, DEFAULT_ARES_URL);
    assert_eq!(cfg.cnb_url, DEFAULT_CNB_URL);
    assert_eq!(cfg.mdcast_url, DEFAULT_MDCAST_URL);
    assert!(cfg.mdcast_token.is_none());
    assert_eq!(cfg.storage, StorageConfig::default());
    assert_eq!(cfg.public.base_url(), "http://localhost:3000");
    assert!(!cfg.registration);
    assert!(!cfg.trust_forwarded);
    assert_eq!(cfg.database_url.expose(), "postgres://x");
}

#[test]
fn bind_is_overridable() {
    let cfg = Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
        ("INVOICEAPP__BIND", "127.0.0.1:9000"),
        ("INVOICEAPP__ARES_URL", "http://127.0.0.1:9001/rest"),
        ("INVOICEAPP__CNB_URL", "http://127.0.0.1:9002/kurz.txt"),
    ]))
    .expect("valid config");
    assert_eq!(cfg.bind, "127.0.0.1:9000");
    assert_eq!(cfg.ares_url, "http://127.0.0.1:9001/rest");
    assert_eq!(cfg.cnb_url, "http://127.0.0.1:9002/kurz.txt");
}

#[test]
fn pdf_settings_are_read() {
    let cfg = Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
        ("INVOICEAPP__MDCAST_URL", "http://127.0.0.1:9003"),
        ("INVOICEAPP__MDCAST_TOKEN", "mdtok"),
        ("INVOICEAPP__STORAGE_DIR", "/srv/data"),
    ]))
    .expect("valid config");
    assert_eq!(cfg.mdcast_url, "http://127.0.0.1:9003");
    assert_eq!(
        cfg.mdcast_token.as_ref().map(|t| t.expose().as_str()),
        Some("mdtok")
    );
    assert_eq!(
        cfg.storage,
        StorageConfig::Fs {
            dir: "/srv/data".into()
        }
    );
    assert!(!format!("{cfg:?}").contains("mdtok"));
}

#[test]
fn pgbouncer_switch() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
    ];
    assert!(
        !Config::from_source(env(&base))
            .expect("default")
            .db_pgbouncer
    );
    let mut pairs = base.to_vec();
    pairs.push(("INVOICEAPP__DB_PGBOUNCER", "true"));
    assert!(Config::from_source(env(&pairs)).expect("set").db_pgbouncer);
    let mut pairs = base.to_vec();
    pairs.push(("INVOICEAPP__DB_PGBOUNCER", "maybe"));
    let err = Config::from_source(env(&pairs)).expect_err("bad");
    assert!(
        format!("{err:#}").contains("INVOICEAPP__DB_PGBOUNCER"),
        "{err:#}"
    );
}

#[test]
fn pool_size() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
    ];
    let cfg = Config::from_source(env(&base)).expect("defaults");
    assert_eq!(cfg.db_max_connections, DEFAULT_MAX_CONNECTIONS);
    let mut pairs = base.to_vec();
    pairs.push(("INVOICEAPP__DB_MAX_CONNECTIONS", " 50 "));
    let cfg = Config::from_source(env(&pairs)).expect("set");
    assert_eq!(cfg.db_max_connections, 50);
    let mut pairs = base.to_vec();
    pairs.push(("INVOICEAPP__DB_MAX_CONNECTIONS", ""));
    let cfg = Config::from_source(env(&pairs)).expect("empty");
    assert_eq!(cfg.db_max_connections, DEFAULT_MAX_CONNECTIONS);
    for bad in ["0", "-1", "lots"] {
        let mut pairs = base.to_vec();
        pairs.push(("INVOICEAPP__DB_MAX_CONNECTIONS", bad));
        let err = Config::from_source(env(&pairs)).expect_err(bad);
        assert!(
            format!("{err:#}").contains("INVOICEAPP__DB_MAX_CONNECTIONS"),
            "{err:#}"
        );
    }
}

#[test]
fn design_cache_size() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
    ];
    let cfg = Config::from_source(env(&base)).expect("defaults");
    assert_eq!(cfg.design_cache_mb, 128);
    let mut pairs = base.to_vec();
    pairs.push(("INVOICEAPP__DESIGN_CACHE_MB", "16"));
    let cfg = Config::from_source(env(&pairs)).expect("set");
    assert_eq!(cfg.design_cache_mb, 16);
    for bad in ["0", "big"] {
        let mut pairs = base.to_vec();
        pairs.push(("INVOICEAPP__DESIGN_CACHE_MB", bad));
        let err = Config::from_source(env(&pairs)).expect_err(bad);
        assert!(
            format!("{err:#}").contains("INVOICEAPP__DESIGN_CACHE_MB"),
            "{err:#}"
        );
    }
}

#[test]
fn mdcast_concurrency() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
    ];
    let cfg = Config::from_source(env(&base)).expect("defaults");
    assert_eq!(cfg.mdcast_concurrency, 4);
    let mut pairs = base.to_vec();
    pairs.push(("INVOICEAPP__MDCAST_CONCURRENCY", "8"));
    assert_eq!(
        Config::from_source(env(&pairs))
            .expect("set")
            .mdcast_concurrency,
        8
    );
    for bad in ["0", "-1", "many"] {
        let mut pairs = base.to_vec();
        pairs.push(("INVOICEAPP__MDCAST_CONCURRENCY", bad));
        let err = Config::from_source(env(&pairs)).expect_err(bad);
        assert!(
            format!("{err:#}").contains("INVOICEAPP__MDCAST_CONCURRENCY"),
            "{err:#}"
        );
    }
}

#[test]
fn empty_pdf_settings_mean_defaults() {
    let cfg = Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
        ("INVOICEAPP__MDCAST_URL", " "),
        ("INVOICEAPP__MDCAST_TOKEN", ""),
        ("INVOICEAPP__STORAGE_DIR", ""),
    ]))
    .expect("valid config");
    assert_eq!(cfg.storage, StorageConfig::default());
    assert_eq!(cfg.mdcast_url, DEFAULT_MDCAST_URL);
    assert!(cfg.mdcast_token.is_none());
}

#[test]
fn invalid_storage_is_rejected() {
    let err = Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
        ("INVOICEAPP__STORAGE_KIND", "s3"),
    ]))
    .expect_err("s3 without a bucket");
    assert!(
        format!("{err:#}").contains("INVOICEAPP__S3__BUCKET"),
        "{err:#}"
    );
}

#[test]
fn smtp_is_optional_and_validated() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
    ];
    assert!(
        Config::from_source(env(&base))
            .expect("valid")
            .smtp
            .is_none()
    );
    let mut with_host = base.to_vec();
    with_host.push(("INVOICEAPP__SMTP__HOST", "smtp.example.com"));
    let err = Config::from_source(env(&with_host)).expect_err("HOST without FROM");
    assert!(
        format!("{err:#}").contains("INVOICEAPP__SMTP__FROM"),
        "{err:#}"
    );
    with_host.push(("INVOICEAPP__SMTP__FROM", "faktury@example.com"));
    let cfg = Config::from_source(env(&with_host)).expect("valid");
    assert_eq!(cfg.smtp.map(|s| s.port), Some(587));
}

#[test]
fn public_url_is_required_and_validated() {
    assert!(Config::from_source(env(&[("INVOICEAPP__DATABASE_URL", "postgres://x")])).is_err());
    let err = Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "invoiceapp.cz"),
    ]))
    .expect_err("no scheme");
    assert!(
        format!("{err:#}").contains("INVOICEAPP__PUBLIC_URL"),
        "{err:#}"
    );
}

#[test]
fn leftover_api_token_is_ignored() {
    Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "https://invoiceapp.cz"),
        ("INVOICEAPP__API_TOKEN", "old"),
    ]))
    .expect("ignored");
}

#[test]
fn switches_are_parsed() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "https://invoiceapp.cz"),
    ];
    let mut with = base.to_vec();
    with.push(("INVOICEAPP__TRUST_FORWARDED", "true"));
    with.push(("INVOICEAPP__REGISTRATION", ""));
    let cfg = Config::from_source(env(&with)).expect("valid");
    assert!(cfg.trust_forwarded);
    assert!(!cfg.registration);
    let mut bad = base.to_vec();
    bad.push(("INVOICEAPP__TRUST_FORWARDED", "yes"));
    assert!(Config::from_source(env(&bad)).is_err());
}

#[test]
fn registration_needs_smtp() {
    let mut with = vec![
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "https://invoiceapp.cz"),
        ("INVOICEAPP__REGISTRATION", "true"),
    ];
    let err = Config::from_source(env(&with)).expect_err("no SMTP");
    assert!(format!("{err:#}").contains("SMTP"), "{err:#}");
    with.push(("INVOICEAPP__SMTP__HOST", "smtp.example.com"));
    with.push(("INVOICEAPP__SMTP__FROM", "faktury@example.com"));
    assert!(Config::from_source(env(&with)).expect("valid").registration);
}

#[test]
fn missing_database_url_is_rejected() {
    assert!(
        Config::from_source(env(&[("INVOICEAPP__PUBLIC_URL", "http://localhost:3000")])).is_err()
    );
    assert!(DbConfig::from_source(env(&[])).is_err());
}

#[test]
fn db_config_needs_only_the_database() {
    let cfg = DbConfig::from_source(env(&[("INVOICEAPP__DATABASE_URL", "postgres://x")]))
        .expect("valid db config");
    assert_eq!(cfg.database_url.expose(), "postgres://x");
}

#[test]
fn debug_never_prints_secrets() {
    let cfg = Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://u:pw@h/db"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
        ("INVOICEAPP__MDCAST_TOKEN", "supersecret"),
    ]))
    .expect("valid config");
    let dbg = format!("{cfg:?}");
    assert!(!dbg.contains("supersecret"));
    assert!(!dbg.contains("pw@"));
}

#[test]
fn abuse_limit_settings() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "https://invoiceapp.cz"),
    ];
    let cfg = Config::from_source(env(&base)).expect("defaults");
    assert!(!cfg.email_disabled);
    assert_eq!(cfg.max_spaces_per_user, DEFAULT_MAX_SPACES_PER_USER);
    let mut with = base.to_vec();
    with.push(("INVOICEAPP__EMAIL_DISABLED", "true"));
    with.push(("INVOICEAPP__MAX_SPACES_PER_USER", "10"));
    let cfg = Config::from_source(env(&with)).expect("set");
    assert!(cfg.email_disabled);
    assert_eq!(cfg.max_spaces_per_user, 10);
    for (key, bad) in [
        ("INVOICEAPP__EMAIL_DISABLED", "yes"),
        ("INVOICEAPP__MAX_SPACES_PER_USER", "0"),
    ] {
        let mut pairs = base.to_vec();
        pairs.push((key, bad));
        let err = Config::from_source(env(&pairs)).expect_err(bad);
        assert!(format!("{err:#}").contains(key), "{err:#}");
    }
}
