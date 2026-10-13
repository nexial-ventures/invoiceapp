//! The OpenID provider over HTTP: discovery (cached a day), the JWKS
//! (`jwks.rs`), the authorization URL and the code exchange (with the
//! client secret and the PKCE verifier).
//! Nothing secret (client secret, code, verifier, ID token) is logged.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use jsonwebtoken::jwk::JwkSet;
use serde::Deserialize;

use super::Fail;
use super::config::GoogleConfig;
use super::idtoken::{self, Expected, Identity, Rejected};
use super::jwks::{self, JwksCache};

const TIMEOUT: Duration = Duration::from_secs(10);
const DISCOVERY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// The shortest interval between two JWKS fetches for an unknown `kid`.
pub const JWKS_REFRESH: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

type Cache = Option<(Arc<Discovery>, Instant)>;

struct Inner {
    cfg: GoogleConfig,
    http: reqwest::Client,
    discovery: Mutex<Cache>,
    jwks: JwksCache,
}

/// Sign in with Google is configured (`AppState::google`).
#[derive(Clone)]
pub struct Google(Arc<Inner>);

#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    error: Option<String>,
}

fn unavailable(e: anyhow::Error) -> Fail {
    tracing::warn!(error = %format!("{e:#}"), "Google sign-in: provider unavailable");
    Fail::Unavailable
}

impl Google {
    pub fn new(cfg: GoogleConfig) -> anyhow::Result<Self> {
        Self::with_jwks_refresh(cfg, JWKS_REFRESH)
    }

    /// [`Google::new`] with another minimum interval between JWKS fetches
    /// for an unknown `kid` (tests).
    pub fn with_jwks_refresh(cfg: GoogleConfig, jwks_refresh: Duration) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("invoiceapp/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("build Google HTTP client")?;
        Ok(Self(Arc::new(Inner {
            cfg,
            http,
            discovery: Mutex::new(None),
            jwks: JwksCache::new(jwks_refresh),
        })))
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.0.discovery.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> anyhow::Result<T> {
        let resp = self
            .0
            .http
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("GET {url}: HTTP {status}");
        }
        resp.json()
            .await
            .with_context(|| format!("GET {url}: unexpected body"))
    }

    async fn discovery(&self) -> Result<Arc<Discovery>, Fail> {
        if let Some((d, at)) = &*self.cache()
            && at.elapsed() < DISCOVERY_TTL
        {
            return Ok(d.clone());
        }
        let url = format!("{}/.well-known/openid-configuration", self.0.cfg.issuer);
        let d: Discovery = self.get_json(&url).await.map_err(unavailable)?;
        if d.issuer.trim_end_matches('/') != self.0.cfg.issuer {
            return Err(unavailable(anyhow::anyhow!(
                "discovery issuer {:?} differs from INVOICEAPP__GOOGLE__ISSUER",
                d.issuer
            )));
        }
        let d = Arc::new(d);
        *self.cache() = Some((d.clone(), Instant::now()));
        Ok(d)
    }

    /// The key set and how long it may be cached (`Cache-Control`).
    async fn fetch_jwks(&self, uri: &str) -> Result<(JwkSet, Duration), Fail> {
        let fetch = async {
            let resp = self
                .0
                .http
                .get(uri)
                .send()
                .await
                .with_context(|| format!("GET {uri}"))?;
            let status = resp.status();
            if !status.is_success() {
                anyhow::bail!("GET {uri}: HTTP {status}");
            }
            let cc = resp
                .headers()
                .get(reqwest::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let set: JwkSet = resp
                .json()
                .await
                .with_context(|| format!("GET {uri}: unexpected body"))?;
            Ok((set, jwks::max_age(cc.as_deref())))
        };
        fetch.await.map_err(unavailable)
    }

    /// The authorization request URL (`prompt=select_account`).
    pub async fn authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        challenge: &str,
        nonce: &str,
    ) -> Result<String, Fail> {
        let d = self.discovery().await?;
        let url = reqwest::Url::parse_with_params(
            &d.authorization_endpoint,
            &[
                ("response_type", "code"),
                ("client_id", self.0.cfg.client_id.as_str()),
                ("redirect_uri", redirect_uri),
                ("scope", "openid email profile"),
                ("state", state),
                ("nonce", nonce),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
                ("prompt", "select_account"),
            ],
        )
        .map_err(|e| unavailable(anyhow::anyhow!("authorization endpoint: {e}")))?;
        Ok(url.into())
    }

    /// Exchange `code` and return the verified identity of its ID token.
    /// The provider unreachable / failing → `unavailable`; a refused code
    /// (HTTP 4xx) or a bad token → `invalid`; `email_verified` not true →
    /// `unverified_email`.
    pub async fn identity(
        &self,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Identity, Fail> {
        let d = self.discovery().await?;
        let id_token = self.exchange(&d, redirect_uri, code, verifier).await?;
        let expected = Expected {
            issuer: &d.issuer,
            client_id: &self.0.cfg.client_id,
            nonce,
        };
        let fetch = || self.fetch_jwks(&d.jwks_uri);
        let mut keys = self.0.jwks.current(fetch).await?;
        let mut refreshed = false;
        loop {
            match idtoken::verify(&id_token, &keys, &expected, chrono::Utc::now().timestamp()) {
                Ok(id) => return Ok(id),
                Err(Rejected::UnknownKid) if !refreshed => {
                    refreshed = true;
                    match self.0.jwks.newer(&keys, fetch).await? {
                        Some(k) => keys = k,
                        None => return Err(rejected("unknown kid (JWKS refreshed recently)")),
                    }
                }
                Err(Rejected::UnknownKid) => return Err(rejected("unknown kid")),
                Err(Rejected::UnverifiedEmail) => return Err(Fail::UnverifiedEmail),
                Err(Rejected::Invalid(reason)) => return Err(rejected(&reason)),
            }
        }
    }

    async fn exchange(
        &self,
        d: &Discovery,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
    ) -> Result<String, Fail> {
        let cfg = &self.0.cfg;
        let resp = self
            .0
            .http
            .post(&d.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", cfg.client_id.as_str()),
                ("client_secret", cfg.client_secret.expose().as_str()),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .context("POST token endpoint")
            .map_err(unavailable)?;
        let status = resp.status();
        if status.is_client_error() {
            let error = resp.json::<TokenError>().await.ok().and_then(|e| e.error);
            tracing::warn!(%status, error = ?error, "Google sign-in: code exchange refused");
            return Err(Fail::Invalid);
        }
        if !status.is_success() {
            return Err(unavailable(anyhow::anyhow!(
                "token endpoint: HTTP {status}"
            )));
        }
        let body: TokenResponse = resp
            .json()
            .await
            .context("token endpoint: unexpected body")
            .map_err(unavailable)?;
        body.id_token.ok_or_else(|| rejected("no id_token"))
    }
}

fn rejected(reason: &str) -> Fail {
    tracing::warn!(reason, "Google sign-in: ID token rejected");
    Fail::Invalid
}
