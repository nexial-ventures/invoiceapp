//! Storage backend settings: `INVOICEAPP__STORAGE_KIND` (`fs` | `s3`),
//! `INVOICEAPP__STORAGE_DIR` for `fs`, `INVOICEAPP__S3__*` for `s3`.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use config::Config as RawConfig;
use serde::Deserialize;

use crate::config::{DEFAULT_STORAGE_DIR, build};
use crate::secret::Secret;

pub const DEFAULT_S3_REGION: &str = "us-east-1";

#[derive(Debug, Clone, PartialEq)]
pub enum StorageConfig {
    Fs { dir: PathBuf },
    S3(S3Config),
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self::Fs {
            dir: PathBuf::from(DEFAULT_STORAGE_DIR),
        }
    }
}

#[derive(Debug, Clone)]
pub struct S3Config {
    /// `None` → AWS.
    pub endpoint: Option<String>,
    pub bucket: String,
    pub region: String,
    pub access_key_id: Secret<String>,
    pub secret_access_key: Secret<String>,
    pub path_style: bool,
}

impl PartialEq for S3Config {
    fn eq(&self, other: &Self) -> bool {
        self.endpoint == other.endpoint
            && self.bucket == other.bucket
            && self.region == other.region
            && self.access_key_id.expose() == other.access_key_id.expose()
            && self.secret_access_key.expose() == other.secret_access_key.expose()
            && self.path_style == other.path_style
    }
}

/// Every value optional: CI and container runtimes pass unset variables
/// through as empty strings, which mean "unset" here.
#[derive(Debug, Default, Deserialize)]
struct Raw {
    storage_kind: Option<String>,
    storage_dir: Option<String>,
    #[serde(default)]
    s3: RawS3,
}

#[derive(Debug, Default, Deserialize)]
struct RawS3 {
    endpoint: Option<String>,
    bucket: Option<String>,
    region: Option<String>,
    access_key_id: Option<Secret<String>>,
    secret_access_key: Option<Secret<String>>,
    path_style: Option<String>,
}

fn set(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn set_secret(v: Option<Secret<String>>) -> Option<Secret<String>> {
    v.filter(|s| !s.expose().trim().is_empty())
}

fn parse_bool(name: &str, v: Option<String>) -> Result<bool> {
    match set(v).map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("false" | "0" | "no") => Ok(false),
        Some("true" | "1" | "yes") => Ok(true),
        Some(other) => bail!("{name} must be true or false, not {other:?}"),
    }
}

impl StorageConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_source(None)
    }

    /// `env` overrides the process environment (used by tests).
    pub fn from_source(env: Option<HashMap<String, String>>) -> Result<Self> {
        Self::from_config(build(env)?)
    }

    pub(crate) fn from_config(raw: RawConfig) -> Result<Self> {
        Self::from_raw(
            raw.try_deserialize()
                .context("invalid storage configuration")?,
        )
    }

    fn from_raw(raw: Raw) -> Result<Self> {
        match set(raw.storage_kind).as_deref().unwrap_or("fs") {
            "fs" => Ok(Self::Fs {
                dir: PathBuf::from(
                    set(raw.storage_dir).unwrap_or_else(|| DEFAULT_STORAGE_DIR.to_string()),
                ),
            }),
            "s3" => Ok(Self::S3(S3Config::from_raw(raw.s3)?)),
            other => bail!("INVOICEAPP__STORAGE_KIND must be fs or s3, not {other:?}"),
        }
    }
}

impl S3Config {
    /// The endpoint requests go to. Path-style: as configured. Virtual-hosted:
    /// the bucket goes into the host (`https://s3.example` →
    /// `https://invoice.s3.example`) unless the host already starts with it.
    pub fn request_endpoint(&self) -> Option<String> {
        let endpoint = self.endpoint.as_deref()?.trim_end_matches('/');
        if self.path_style {
            return Some(endpoint.to_string());
        }
        let prefix = format!("{}.", self.bucket);
        match endpoint.split_once("://") {
            Some((scheme, host)) if !host.starts_with(&prefix) => {
                Some(format!("{scheme}://{prefix}{host}"))
            }
            _ => Some(endpoint.to_string()),
        }
    }

    fn from_raw(raw: RawS3) -> Result<Self> {
        let endpoint = set(raw.endpoint);
        if let Some(e) = &endpoint
            && !(e.starts_with("http://") || e.starts_with("https://"))
        {
            bail!("INVOICEAPP__S3__ENDPOINT must start with http:// or https://");
        }
        Ok(Self {
            endpoint,
            bucket: set(raw.bucket).context("INVOICEAPP__S3__BUCKET is required for s3 storage")?,
            region: set(raw.region).unwrap_or_else(|| DEFAULT_S3_REGION.to_string()),
            access_key_id: set_secret(raw.access_key_id)
                .context("INVOICEAPP__S3__ACCESS_KEY_ID is required for s3 storage")?,
            secret_access_key: set_secret(raw.secret_access_key)
                .context("INVOICEAPP__S3__SECRET_ACCESS_KEY is required for s3 storage")?,
            path_style: parse_bool("INVOICEAPP__S3__PATH_STYLE", raw.path_style)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pairs: &[(&str, &str)]) -> Result<StorageConfig> {
        StorageConfig::from_source(Some(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ))
    }

    const S3: [(&str, &str); 6] = [
        ("INVOICEAPP__STORAGE_KIND", "s3"),
        ("INVOICEAPP__S3__ENDPOINT", "https://s3.example.test"),
        ("INVOICEAPP__S3__BUCKET", "invoice"),
        ("INVOICEAPP__S3__REGION", "garage"),
        ("INVOICEAPP__S3__ACCESS_KEY_ID", "GKkey"),
        ("INVOICEAPP__S3__SECRET_ACCESS_KEY", "s3cret"),
    ];

    #[test]
    fn fs_is_the_default() {
        let dir = PathBuf::from(DEFAULT_STORAGE_DIR);
        assert_eq!(cfg(&[]).expect("default"), StorageConfig::Fs { dir });
        let empty = cfg(&[
            ("INVOICEAPP__STORAGE_KIND", ""),
            ("INVOICEAPP__STORAGE_DIR", " "),
        ]);
        assert_eq!(
            empty.expect("empty values"),
            StorageConfig::Fs {
                dir: PathBuf::from(DEFAULT_STORAGE_DIR)
            }
        );
        let custom = cfg(&[("INVOICEAPP__STORAGE_DIR", "/srv/data")]).expect("dir");
        assert_eq!(
            custom,
            StorageConfig::Fs {
                dir: PathBuf::from("/srv/data")
            }
        );
    }

    #[test]
    fn s3_settings_are_read() {
        let mut pairs = S3.to_vec();
        pairs.push(("INVOICEAPP__S3__PATH_STYLE", "true"));
        let StorageConfig::S3(s3) = cfg(&pairs).expect("s3") else {
            panic!("expected s3");
        };
        assert_eq!(s3.endpoint.as_deref(), Some("https://s3.example.test"));
        assert_eq!(
            (s3.bucket.as_str(), s3.region.as_str()),
            ("invoice", "garage")
        );
        assert_eq!(s3.access_key_id.expose(), "GKkey");
        assert!(s3.path_style);
        assert!(!format!("{s3:?}").contains("s3cret"));
    }

    #[test]
    fn s3_defaults() {
        let pairs: Vec<_> = S3
            .iter()
            .copied()
            .filter(|(k, _)| !k.ends_with("ENDPOINT") && !k.ends_with("REGION"))
            .chain([
                ("INVOICEAPP__S3__REGION", ""),
                ("INVOICEAPP__S3__PATH_STYLE", ""),
            ])
            .collect();
        let StorageConfig::S3(s3) = cfg(&pairs).expect("s3") else {
            panic!("expected s3");
        };
        assert_eq!(s3.endpoint, None);
        assert_eq!(s3.region, DEFAULT_S3_REGION);
        assert!(!s3.path_style);
    }

    #[test]
    fn virtual_hosted_endpoint_carries_the_bucket() {
        let StorageConfig::S3(mut s3) = cfg(&S3).expect("s3") else {
            panic!("expected s3");
        };
        assert_eq!(
            s3.request_endpoint().as_deref(),
            Some("https://invoice.s3.example.test")
        );
        s3.endpoint = Some("https://invoice.s3.example.test/".into());
        assert_eq!(
            s3.request_endpoint().as_deref(),
            Some("https://invoice.s3.example.test")
        );
        s3.path_style = true;
        s3.endpoint = Some("http://127.0.0.1:3900".into());
        assert_eq!(
            s3.request_endpoint().as_deref(),
            Some("http://127.0.0.1:3900")
        );
        s3.endpoint = None;
        assert_eq!(s3.request_endpoint(), None);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let err = cfg(&[("INVOICEAPP__STORAGE_KIND", "ftp")]).expect_err("kind");
        assert!(
            err.to_string().contains("INVOICEAPP__STORAGE_KIND"),
            "{err}"
        );
        for missing in ["BUCKET", "ACCESS_KEY_ID", "SECRET_ACCESS_KEY"] {
            let pairs: Vec<_> = S3
                .iter()
                .copied()
                .filter(|(k, _)| !k.ends_with(missing))
                .collect();
            let err = cfg(&pairs).expect_err(missing);
            assert!(format!("{err:#}").contains(missing), "{err:#}");
        }
        let mut pairs = S3.to_vec();
        pairs.push(("INVOICEAPP__S3__PATH_STYLE", "maybe"));
        assert!(cfg(&pairs).is_err());
        let mut pairs = S3.to_vec();
        pairs[1] = ("INVOICEAPP__S3__ENDPOINT", "s3.example.test");
        let err = cfg(&pairs).expect_err("endpoint scheme");
        assert!(
            err.to_string().contains("INVOICEAPP__S3__ENDPOINT"),
            "{err}"
        );
    }
}
