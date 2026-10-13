//! `INVOICEAPP__GOOGLE__*`: client id + secret (both or neither) and the
//! issuer (discovery base, default Google).

use anyhow::{Context, Result, bail};
use config::Config as RawConfig;
use serde::Deserialize;

use crate::secret::Secret;

pub const DEFAULT_ISSUER: &str = "https://accounts.google.com";

/// Sign in with Google is enabled.
#[derive(Debug, Clone)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: Secret<String>,
    /// Without a trailing `/`.
    pub issuer: String,
}

/// Every value optional: unset variables may arrive as empty strings.
#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    google: RawGoogle,
}

#[derive(Debug, Default, Deserialize)]
struct RawGoogle {
    client_id: Option<String>,
    client_secret: Option<Secret<String>>,
    issuer: Option<String>,
}

fn set(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl GoogleConfig {
    /// `None` = disabled (neither id nor secret); only one of them → error.
    pub(crate) fn from_config(raw: RawConfig) -> Result<Option<Self>> {
        let raw: Raw = raw
            .try_deserialize()
            .context("invalid Google configuration")?;
        Self::from_raw(raw.google)
    }

    fn from_raw(raw: RawGoogle) -> Result<Option<Self>> {
        let secret = raw
            .client_secret
            .map(|s| s.expose().trim().to_string())
            .filter(|s| !s.is_empty());
        let (client_id, client_secret) = match (set(raw.client_id), secret) {
            (Some(id), Some(secret)) => (id, Secret::new(secret)),
            (None, None) => return Ok(None),
            _ => bail!(
                "INVOICEAPP__GOOGLE__CLIENT_ID and INVOICEAPP__GOOGLE__CLIENT_SECRET must be set together"
            ),
        };
        let issuer = set(raw.issuer).unwrap_or_else(|| DEFAULT_ISSUER.to_string());
        let issuer = issuer.trim_end_matches('/').to_string();
        check_issuer(&issuer)?;
        Ok(Some(Self {
            client_id,
            client_secret,
            issuer,
        }))
    }
}

/// `https://…`, or `http://` to a loopback host (a local mock provider).
fn check_issuer(issuer: &str) -> Result<()> {
    let url = reqwest::Url::parse(issuer)
        .with_context(|| format!("INVOICEAPP__GOOGLE__ISSUER is not a URL: {issuer:?}"))?;
    let host = url.host_str().unwrap_or_default();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    let ok = match url.scheme() {
        "https" => !host.is_empty(),
        "http" => loopback,
        _ => false,
    };
    if !ok || url.query().is_some() || url.fragment().is_some() {
        bail!("INVOICEAPP__GOOGLE__ISSUER must be an https URL (http only for a loopback host)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: Option<&str>, secret: Option<&str>, issuer: Option<&str>) -> RawGoogle {
        RawGoogle {
            client_id: id.map(str::to_string),
            client_secret: secret.map(|s| Secret::new(s.to_string())),
            issuer: issuer.map(str::to_string),
        }
    }

    #[test]
    fn both_or_neither() {
        assert!(
            GoogleConfig::from_raw(raw(None, None, None))
                .unwrap()
                .is_none()
        );
        assert!(
            GoogleConfig::from_raw(raw(Some(" "), Some(""), None))
                .unwrap()
                .is_none()
        );
        for (id, secret) in [
            (Some("id"), None),
            (None, Some("secret")),
            (Some("id"), Some(" ")),
        ] {
            let err = GoogleConfig::from_raw(raw(id, secret, None)).expect_err("only one");
            assert!(err.to_string().contains("set together"), "{err}");
        }
        let cfg = GoogleConfig::from_raw(raw(Some("id"), Some("secret"), None))
            .unwrap()
            .expect("enabled");
        assert_eq!(cfg.issuer, DEFAULT_ISSUER);
        assert_eq!(cfg.client_id, "id");
        assert!(!format!("{cfg:?}").contains("secret\""));
    }

    #[test]
    fn issuer_rules() {
        let ok = |i: &str| GoogleConfig::from_raw(raw(Some("id"), Some("s"), Some(i)));
        assert_eq!(
            ok("http://127.0.0.1:4000/").unwrap().expect("on").issuer,
            "http://127.0.0.1:4000"
        );
        assert!(ok("http://localhost:4000").is_ok());
        assert!(ok("https://idp.example.com/realm").is_ok());
        for bad in [
            "http://idp.example.com",
            "ftp://x",
            "nope",
            "https://x.cz/?a=1",
        ] {
            assert!(ok(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_the_environment() {
        let env = |pairs: &[(&str, &str)]| {
            Some(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )
        };
        let raw = crate::config::build(env(&[
            ("INVOICEAPP__GOOGLE__CLIENT_ID", "cid"),
            ("INVOICEAPP__GOOGLE__CLIENT_SECRET", "cs"),
        ]))
        .unwrap();
        let cfg = GoogleConfig::from_config(raw).unwrap().expect("enabled");
        assert_eq!(
            (cfg.client_id.as_str(), cfg.client_secret.expose().as_str()),
            ("cid", "cs")
        );
        let raw = crate::config::build(env(&[("INVOICEAPP__GOOGLE__CLIENT_ID", "cid")])).unwrap();
        assert!(GoogleConfig::from_config(raw).is_err());
    }
}
