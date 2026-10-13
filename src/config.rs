//! Runtime configuration from `INVOICEAPP__*` environment variables.

use anyhow::{Context, Result, bail};
use config::{Config as RawConfig, Environment};
use sea_orm::ConnectOptions;
use serde::Deserialize;
use std::collections::HashMap;

use crate::ares::DEFAULT_ARES_URL;
use crate::auth::google::GoogleConfig;
use crate::auth::host::PublicUrl;
use crate::cnb::DEFAULT_CNB_URL;
use crate::db::DEFAULT_MAX_CONNECTIONS;
use crate::email::SmtpConfig;
use crate::pdf::DEFAULT_MDCAST_URL;
use crate::pdf::design_cache::DEFAULT_LIMIT_MB as DEFAULT_DESIGN_CACHE_MB;
use crate::pdf::limits::DEFAULT_CONCURRENCY as DEFAULT_MDCAST_CONCURRENCY;
use crate::secret::{Secret, SecretKey};
use crate::storage::StorageConfig;

pub const DEFAULT_BIND: &str = "0.0.0.0:3000";
pub const DEFAULT_STORAGE_DIR: &str = "./data";
pub const DEFAULT_MAX_SPACES_PER_USER: u32 = 3;

/// Full server configuration (`invoiceapp serve`).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub database_url: Secret<String>,
    /// `INVOICEAPP__PUBLIC_URL` (required): scheme, base host, optional port.
    #[serde(rename = "public_url")]
    raw_public_url: String,
    #[serde(skip)]
    pub public: PublicUrl,
    /// `INVOICEAPP__REGISTRATION` (`true` / `false`, default `false`).
    #[serde(default, rename = "registration")]
    raw_registration: Option<String>,
    #[serde(skip)]
    pub registration: bool,
    /// `INVOICEAPP__EMAIL_DISABLED` (default `false`): refuse every outbound e-mail.
    #[serde(default, rename = "email_disabled")]
    raw_email_disabled: Option<String>,
    #[serde(skip)]
    pub email_disabled: bool,
    /// `INVOICEAPP__MAX_SPACES_PER_USER` (default 3): spaces a user may create and own.
    #[serde(default, rename = "max_spaces_per_user")]
    raw_max_spaces_per_user: Option<String>,
    #[serde(skip)]
    pub max_spaces_per_user: u32,
    /// `INVOICEAPP__TRUST_FORWARDED` (default `false`).
    #[serde(default, rename = "trust_forwarded")]
    raw_trust_forwarded: Option<String>,
    #[serde(skip)]
    pub trust_forwarded: bool,
    /// `INVOICEAPP__SECRET_KEY` (required): 32 bytes, base64.
    #[serde(default, rename = "secret_key")]
    raw_secret_key: Option<Secret<String>>,
    #[serde(skip)]
    pub secret_key: SecretKey,
    pub bind: String,
    /// ARES REST root (`INVOICEAPP__ARES_URL`).
    pub ares_url: String,
    /// ČNB `denni_kurz.txt` URL (`INVOICEAPP__CNB_URL`).
    pub cnb_url: String,
    /// mdcast render service root (`INVOICEAPP__MDCAST_URL`; empty → default).
    pub mdcast_url: String,
    /// Optional Bearer token for mdcast (`INVOICEAPP__MDCAST_TOKEN`).
    pub mdcast_token: Option<Secret<String>>,
    /// `INVOICEAPP__MDCAST_CONCURRENCY`: concurrent mdcast renders (default 4).
    #[serde(default, rename = "mdcast_concurrency")]
    raw_mdcast_concurrency: Option<String>,
    #[serde(skip)]
    pub mdcast_concurrency: u32,
    /// `INVOICEAPP__DB_MAX_CONNECTIONS`: pool size (default 20).
    #[serde(default, rename = "db_max_connections")]
    raw_db_max_connections: Option<String>,
    #[serde(skip)]
    pub db_max_connections: u32,
    /// `INVOICEAPP__DB_PGBOUNCER`: behind a transaction-pooling PgBouncer.
    #[serde(default, rename = "db_pgbouncer")]
    raw_db_pgbouncer: Option<String>,
    #[serde(skip)]
    pub db_pgbouncer: bool,
    /// `INVOICEAPP__DESIGN_CACHE_MB`: design file cache bound (default 128).
    #[serde(default, rename = "design_cache_mb")]
    raw_design_cache_mb: Option<String>,
    #[serde(skip)]
    pub design_cache_mb: u32,
    /// `INVOICEAPP__STORAGE_KIND` / `INVOICEAPP__STORAGE_DIR` / `INVOICEAPP__S3__*`.
    #[serde(skip)]
    pub storage: StorageConfig,
    /// `INVOICEAPP__SMTP__*`; `None` → e-mail not configured.
    #[serde(skip)]
    pub smtp: Option<SmtpConfig>,
    /// `INVOICEAPP__GOOGLE__*`; `None` → Sign in with Google disabled.
    #[serde(skip)]
    pub google: Option<GoogleConfig>,
    /// Removed setting, read only to refuse a start that would silently fall
    /// back to the default design (archives are immutable).
    #[serde(default, rename = "design_dir")]
    removed_design_dir: Option<String>,
}

/// The subset `invoiceapp migrate` (and `invoiceapp design`) needs.
#[derive(Debug, Clone, Deserialize)]
pub struct DbConfig {
    pub database_url: Secret<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_source(None)
    }

    /// `env` overrides the process environment (used by tests).
    pub fn from_source(env: Option<HashMap<String, String>>) -> Result<Self> {
        let raw = build(env)?;
        let mut cfg: Self = raw.clone().try_deserialize().context(
            "invalid configuration (INVOICEAPP__DATABASE_URL and INVOICEAPP__PUBLIC_URL are required)",
        )?;
        cfg.storage = StorageConfig::from_config(raw.clone())?;
        cfg.google = GoogleConfig::from_config(raw.clone())?;
        cfg.smtp = SmtpConfig::from_config(raw)?;
        cfg.public = PublicUrl::parse(&cfg.raw_public_url)?;
        cfg.registration = flag("INVOICEAPP__REGISTRATION", cfg.raw_registration.as_deref())?;
        cfg.email_disabled = flag(
            "INVOICEAPP__EMAIL_DISABLED",
            cfg.raw_email_disabled.as_deref(),
        )?;
        cfg.max_spaces_per_user = positive(
            "INVOICEAPP__MAX_SPACES_PER_USER",
            cfg.raw_max_spaces_per_user.as_deref(),
            DEFAULT_MAX_SPACES_PER_USER,
        )?;
        cfg.trust_forwarded = flag(
            "INVOICEAPP__TRUST_FORWARDED",
            cfg.raw_trust_forwarded.as_deref(),
        )?;
        cfg.secret_key = secret_key(cfg.raw_secret_key.as_ref())?;
        cfg.db_max_connections = positive(
            "INVOICEAPP__DB_MAX_CONNECTIONS",
            cfg.raw_db_max_connections.as_deref(),
            DEFAULT_MAX_CONNECTIONS,
        )?;
        cfg.db_pgbouncer = flag("INVOICEAPP__DB_PGBOUNCER", cfg.raw_db_pgbouncer.as_deref())?;
        cfg.mdcast_concurrency = positive(
            "INVOICEAPP__MDCAST_CONCURRENCY",
            cfg.raw_mdcast_concurrency.as_deref(),
            DEFAULT_MDCAST_CONCURRENCY,
        )?;
        cfg.design_cache_mb = positive(
            "INVOICEAPP__DESIGN_CACHE_MB",
            cfg.raw_design_cache_mb.as_deref(),
            DEFAULT_DESIGN_CACHE_MB,
        )?;
        let cfg = cfg.normalized();
        cfg.validate()?;
        Ok(cfg)
    }

    /// The `invoiceapp serve` request pool.
    pub fn pool_options(&self) -> ConnectOptions {
        crate::db::pool_options(
            self.database_url.expose(),
            self.db_max_connections,
            self.db_pgbouncer,
        )
    }

    /// CI and container runtimes pass unset variables through as empty
    /// strings; those mean "use the default", not "the empty value".
    fn normalized(mut self) -> Self {
        if self.mdcast_url.trim().is_empty() {
            self.mdcast_url = DEFAULT_MDCAST_URL.to_string();
        }
        if self
            .mdcast_token
            .as_ref()
            .is_some_and(|t| t.expose().trim().is_empty())
        {
            self.mdcast_token = None;
        }
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.registration && self.smtp.is_none() {
            bail!(
                "INVOICEAPP__REGISTRATION=true needs SMTP (INVOICEAPP__SMTP__HOST / FROM): \
                 verification and reset e-mails go through it"
            );
        }
        if self
            .removed_design_dir
            .as_ref()
            .is_some_and(|d| !d.trim().is_empty())
        {
            bail!(
                "INVOICEAPP__DESIGN_DIR was removed: the design now lives in the storage under design/. \
                 Copy it with `invoiceapp storage migrate --from-dir <INVOICEAPP__STORAGE_DIR> --design-dir <dir>` \
                 (or `invoiceapp design push <dir>`), then unset INVOICEAPP__DESIGN_DIR"
            );
        }
        validate_database_url(&self.database_url)
    }
}

impl DbConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_source(None)
    }

    pub fn from_source(env: Option<HashMap<String, String>>) -> Result<Self> {
        let cfg: Self = build(env)?
            .try_deserialize()
            .context("invalid configuration (INVOICEAPP__DATABASE_URL is required)")?;
        validate_database_url(&cfg.database_url)?;
        Ok(cfg)
    }
}

/// A boolean switch: unset or empty → `false`.
fn flag(name: &str, raw: Option<&str>) -> Result<bool> {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") | Some("false") | Some("0") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(_) => bail!("{name} must be true or false"),
    }
}

/// A count of at least 1: unset or empty → `default`.
fn positive(name: &str, raw: Option<&str>, default: u32) -> Result<u32> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(default),
        Some(s) => match s.parse::<u32>() {
            Ok(n) if n >= 1 => Ok(n),
            _ => bail!("{name} must be a whole number of at least 1"),
        },
    }
}

/// Missing, empty, not base64 or not 32 bytes → the start is refused.
fn secret_key(raw: Option<&Secret<String>>) -> Result<SecretKey> {
    match raw.map(|s| s.expose().trim()).filter(|s| !s.is_empty()) {
        Some(b64) => SecretKey::parse(b64),
        None => bail!(
            "INVOICEAPP__SECRET_KEY is required: 32 random bytes, base64 (`openssl rand -base64 32`)"
        ),
    }
}

fn validate_database_url(url: &Secret<String>) -> Result<()> {
    if url.expose().trim().is_empty() {
        bail!("INVOICEAPP__DATABASE_URL must not be empty");
    }
    Ok(())
}

/// The prefix before the InvoiceApp rename (#53).
const OLD_PREFIX: &str = "INVOICE__";

/// Nothing is read from the old prefix any more: a leftover would silently
/// fall back to a default (or fail on a "missing" setting that is set), so
/// every command that reads the configuration refuses it up front.
fn reject_old_prefix(env: Option<&HashMap<String, String>>) -> Result<()> {
    let keys: Vec<String> = match env {
        Some(map) => map.keys().cloned().collect(),
        None => std::env::vars_os()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect(),
    };
    let n = OLD_PREFIX.len();
    let mut renames: Vec<String> = keys
        .iter()
        .filter_map(|k| {
            let rest = k.get(n..)?;
            k.get(..n)?.eq_ignore_ascii_case(OLD_PREFIX).then(|| {
                if rest.eq_ignore_ascii_case("DESIGN_DIR") {
                    format!(
                        "{k} (removed: copy the design with `invoiceapp storage migrate --design-dir`, then unset it)"
                    )
                } else {
                    format!("{k} → INVOICEAPP__{rest}")
                }
            })
        })
        .collect();
    if renames.is_empty() {
        return Ok(());
    }
    renames.sort();
    bail!(
        "the environment prefix INVOICE__ was renamed to INVOICEAPP__ and is no longer read; rename: {}",
        renames.join(", ")
    )
}

pub(crate) fn build(env: Option<HashMap<String, String>>) -> Result<RawConfig> {
    reject_old_prefix(env.as_ref())?;
    RawConfig::builder()
        .set_default("bind", DEFAULT_BIND)
        .and_then(|b| b.set_default("ares_url", DEFAULT_ARES_URL))
        .and_then(|b| b.set_default("cnb_url", DEFAULT_CNB_URL))
        .and_then(|b| b.set_default("mdcast_url", DEFAULT_MDCAST_URL))
        .context("set config defaults")?
        .add_source(
            Environment::with_prefix("INVOICEAPP")
                .prefix_separator("__")
                .separator("__")
                .source(env),
        )
        .build()
        .context("read configuration from environment")
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "config_legacy_tests.rs"]
mod legacy_tests;
