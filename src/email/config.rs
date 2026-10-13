//! SMTP settings: `INVOICEAPP__SMTP__HOST`, `PORT`, `TLS`, `USERNAME`,
//! `PASSWORD`, `FROM`. No host → e-mail not configured.

use std::collections::HashMap;

use anyhow::{Context as _, Result, bail};
use config::Config as RawConfig;
use lettre::message::Mailbox;
use serde::Deserialize;

use crate::config::build;
use crate::secret::Secret;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// Plain connection upgraded with STARTTLS (required).
    StartTls,
    /// Implicit TLS from the first byte.
    Tls,
    /// Plain text: tests and a local relay only.
    None,
}

impl TlsMode {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "starttls" => Some(Self::StartTls),
            "tls" => Some(Self::Tls),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::StartTls => 587,
            Self::Tls => 465,
            Self::None => 25,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub credentials: Option<(String, Secret<String>)>,
    pub from: Mailbox,
}

/// Every value optional: unset variables may arrive as empty strings.
#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    smtp: RawSmtp,
}

#[derive(Debug, Default, Deserialize)]
struct RawSmtp {
    host: Option<String>,
    port: Option<String>,
    tls: Option<String>,
    username: Option<String>,
    password: Option<Secret<String>>,
    from: Option<String>,
}

/// `localhost`, `127.0.0.0/8` or `::1` (brackets allowed).
fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn set(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl SmtpConfig {
    /// `env` overrides the process environment (used by tests).
    pub fn from_source(env: Option<HashMap<String, String>>) -> Result<Option<Self>> {
        Self::from_config(build(env)?)
    }

    pub(crate) fn from_config(raw: RawConfig) -> Result<Option<Self>> {
        let raw: Raw = raw
            .try_deserialize()
            .context("invalid SMTP configuration")?;
        Self::from_raw(raw.smtp)
    }

    fn from_raw(raw: RawSmtp) -> Result<Option<Self>> {
        let tls = match set(raw.tls) {
            None => TlsMode::StartTls,
            Some(t) => TlsMode::parse(&t).with_context(|| {
                format!("INVOICEAPP__SMTP__TLS must be starttls, tls or none, not {t:?}")
            })?,
        };
        let port = set(raw.port)
            .map(|p| {
                p.parse::<u16>().ok().filter(|p| *p > 0).with_context(|| {
                    format!("INVOICEAPP__SMTP__PORT must be a port number, not {p:?}")
                })
            })
            .transpose()?;
        let username = set(raw.username);
        let password = raw.password.filter(|p| !p.expose().is_empty());
        let credentials = match (username, password) {
            (Some(u), Some(p)) => Some((u, p)),
            (None, None) => None,
            _ => bail!(
                "INVOICEAPP__SMTP__USERNAME and INVOICEAPP__SMTP__PASSWORD must be set together"
            ),
        };
        let from = set(raw.from);
        let Some(host) = set(raw.host) else {
            return Ok(None);
        };
        if credentials.is_some() && tls == TlsMode::None && !is_loopback(&host) {
            bail!(
                "INVOICEAPP__SMTP__TLS=none would send INVOICEAPP__SMTP__USERNAME / PASSWORD in plain text \
                 to {host}: use starttls or tls, or a loopback relay (localhost, 127.0.0.0/8, ::1)"
            );
        }
        let from =
            from.context("INVOICEAPP__SMTP__FROM is required when INVOICEAPP__SMTP__HOST is set")?;
        let from: Mailbox = from.parse().map_err(|e| {
            anyhow::anyhow!(
                "INVOICEAPP__SMTP__FROM must be addr@domain or Name <addr@domain>, not {from:?}: {e}"
            )
        })?;
        Ok(Some(Self {
            port: port.unwrap_or(tls.default_port()),
            host,
            tls,
            credentials,
            from,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pairs: &[(&str, &str)]) -> Result<Option<SmtpConfig>> {
        SmtpConfig::from_source(Some(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ))
    }

    #[test]
    fn unset_host_means_not_configured() {
        assert!(cfg(&[]).expect("valid").is_none());
        assert!(
            cfg(&[
                ("INVOICEAPP__SMTP__HOST", " "),
                ("INVOICEAPP__SMTP__FROM", "")
            ])
            .expect("valid")
            .is_none()
        );
    }

    #[test]
    fn defaults_follow_the_tls_mode() {
        let c = cfg(&[
            ("INVOICEAPP__SMTP__HOST", "smtp.example.com"),
            ("INVOICEAPP__SMTP__FROM", "Firma <faktury@example.com>"),
        ])
        .expect("valid")
        .expect("configured");
        assert_eq!((c.tls, c.port), (TlsMode::StartTls, 587));
        assert_eq!(c.from.email.to_string(), "faktury@example.com");
        assert_eq!(c.from.name.as_deref(), Some("Firma"));
        assert!(c.credentials.is_none());
        for (mode, tls, port) in [("tls", TlsMode::Tls, 465), ("NONE", TlsMode::None, 25)] {
            let c = cfg(&[
                ("INVOICEAPP__SMTP__HOST", "h"),
                ("INVOICEAPP__SMTP__FROM", "a@b.cz"),
                ("INVOICEAPP__SMTP__TLS", mode),
            ])
            .expect("valid")
            .expect("configured");
            assert_eq!((c.tls, c.port), (tls, port));
        }
        let c = cfg(&[
            ("INVOICEAPP__SMTP__HOST", "h"),
            ("INVOICEAPP__SMTP__FROM", "a@b.cz"),
            ("INVOICEAPP__SMTP__PORT", "2525"),
            ("INVOICEAPP__SMTP__USERNAME", "u"),
            ("INVOICEAPP__SMTP__PASSWORD", "secret-pw"),
        ])
        .expect("valid")
        .expect("configured");
        assert_eq!(c.port, 2525);
        assert_eq!(
            c.credentials
                .as_ref()
                .map(|(u, p)| (u.as_str(), p.expose().as_str())),
            Some(("u", "secret-pw"))
        );
        assert!(!format!("{c:?}").contains("secret-pw"));
    }

    #[test]
    fn credentials_in_plain_text_only_to_loopback() {
        let with = |host: &str| {
            cfg(&[
                ("INVOICEAPP__SMTP__HOST", host),
                ("INVOICEAPP__SMTP__FROM", "a@b.cz"),
                ("INVOICEAPP__SMTP__TLS", "none"),
                ("INVOICEAPP__SMTP__USERNAME", "u"),
                ("INVOICEAPP__SMTP__PASSWORD", "p"),
            ])
        };
        for host in [
            "localhost",
            "LOCALHOST",
            "127.0.0.1",
            "127.1.2.3",
            "::1",
            "[::1]",
        ] {
            assert!(with(host).is_ok(), "{host}");
        }
        for host in ["smtp.example.com", "10.0.0.1", "128.0.0.1", "::2"] {
            let err = with(host).expect_err("plain-text credentials");
            assert!(
                format!("{err:#}").contains("INVOICEAPP__SMTP__TLS=none"),
                "{err:#}"
            );
        }
        // Without credentials a remote plain relay is allowed.
        assert!(
            cfg(&[
                ("INVOICEAPP__SMTP__HOST", "relay.example.com"),
                ("INVOICEAPP__SMTP__FROM", "a@b.cz"),
                ("INVOICEAPP__SMTP__TLS", "none"),
            ])
            .is_ok()
        );
    }

    #[test]
    fn invalid_settings_refuse_the_start() {
        let cases: [(&[(&str, &str)], &str); 5] = [
            (
                &[("INVOICEAPP__SMTP__HOST", "h")],
                "INVOICEAPP__SMTP__FROM is required",
            ),
            (
                &[
                    ("INVOICEAPP__SMTP__HOST", "h"),
                    ("INVOICEAPP__SMTP__FROM", "not an address"),
                ],
                "INVOICEAPP__SMTP__FROM must be",
            ),
            (
                &[
                    ("INVOICEAPP__SMTP__HOST", "h"),
                    ("INVOICEAPP__SMTP__FROM", "a@b.cz"),
                    ("INVOICEAPP__SMTP__TLS", "ssl"),
                ],
                "INVOICEAPP__SMTP__TLS",
            ),
            (
                &[
                    ("INVOICEAPP__SMTP__HOST", "h"),
                    ("INVOICEAPP__SMTP__FROM", "a@b.cz"),
                    ("INVOICEAPP__SMTP__PORT", "x"),
                ],
                "INVOICEAPP__SMTP__PORT",
            ),
            (
                &[
                    ("INVOICEAPP__SMTP__HOST", "h"),
                    ("INVOICEAPP__SMTP__FROM", "a@b.cz"),
                    ("INVOICEAPP__SMTP__USERNAME", "u"),
                ],
                "set together",
            ),
        ];
        for (env, msg) in cases {
            let err = cfg(env).expect_err("invalid");
            assert!(format!("{err:#}").contains(msg), "{err:#}");
        }
    }
}
