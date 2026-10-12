//! Request host → context: the base host of `INVOICE__PUBLIC_URL`, the host
//! of an existing space (`{slug}.{base host}`), or unknown (404 for every
//! `/api` route). Also `GET /api/context`.

use anyhow::{Result, bail};
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, header};
use axum::middleware::Next;
use axum::response::Response;
use serde::Serialize;
use utoipa::ToSchema;

use crate::app::AppState;
use crate::error::{AppError, ErrorBody};
use crate::space::entity::space;
use crate::space::{SpaceId, repo, slug};

/// `INVOICE__PUBLIC_URL`, parsed: scheme, base host, optional port.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublicUrl {
    pub https: bool,
    /// Lowercased, no port.
    pub host: String,
    pub port: Option<u16>,
}

impl PublicUrl {
    /// `http(s)://host[:port]` with an optional trailing `/`; no path,
    /// query, user info or IP-literal host.
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        let (https, rest) = if let Some(r) = raw.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = raw.strip_prefix("http://") {
            (false, r)
        } else {
            bail!("INVOICE__PUBLIC_URL must start with http:// or https://");
        };
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.is_empty() || authority.contains(['/', '?', '#', '@', '[']) {
            bail!("INVOICE__PUBLIC_URL must be scheme://host[:port] without a path");
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => match p.parse::<u16>() {
                Ok(port) => (h, Some(port)),
                Err(_) => bail!("INVOICE__PUBLIC_URL has an invalid port"),
            },
            None => (authority, None),
        };
        let host = host.to_ascii_lowercase();
        let label_ok =
            |l: &str| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !host.split('.').all(label_ok) {
            bail!("INVOICE__PUBLIC_URL has an invalid host");
        }
        Ok(Self { https, host, port })
    }

    fn scheme(&self) -> &'static str {
        if self.https { "https" } else { "http" }
    }

    fn port_suffix(&self) -> String {
        self.port.map(|p| format!(":{p}")).unwrap_or_default()
    }

    /// `{scheme}://{base host}{:port}` — `baseUrl` of `GET /api/context`.
    pub fn base_url(&self) -> String {
        format!("{}://{}{}", self.scheme(), self.host, self.port_suffix())
    }

    /// The URL of a space's host.
    pub fn space_url(&self, slug: &str) -> String {
        format!(
            "{}://{slug}.{}{}",
            self.scheme(),
            self.host,
            self.port_suffix()
        )
    }

    /// The origin a browser sends for a page on `host` (the request's own
    /// `Host`, port included): what the CSRF check compares `Origin` with.
    pub fn origin_of(&self, host: &str) -> String {
        format!("{}://{}", self.scheme(), host.to_ascii_lowercase())
    }

    /// Classify a request host (port ignored, case-insensitive).
    pub fn classify(&self, host: &str) -> HostKind {
        let name = strip_port(host).to_ascii_lowercase();
        let name = name.strip_suffix('.').unwrap_or(&name);
        if name == self.host {
            return HostKind::Base;
        }
        match name
            .strip_suffix(self.host.as_str())
            .and_then(|s| s.strip_suffix('.'))
        {
            Some(label) if !label.contains('.') && slug::plausible(label) => {
                HostKind::Slug(label.to_string())
            }
            _ => HostKind::Unknown,
        }
    }
}

fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        // An IPv6 literal is never the base host or a space host.
        return host;
    }
    host.rsplit_once(':').map_or(host, |(h, _)| h)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKind {
    Base,
    Slug(String),
    Unknown,
}

/// The request's own host as the client named it: the URI authority
/// (HTTP/2) or the `Host` header.
pub fn request_host(uri: &axum::http::Uri, headers: &HeaderMap) -> Option<String> {
    uri.authority()
        .map(|a| a.as_str().to_string())
        .or_else(|| {
            headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        })
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
}

/// What the host middleware puts into the request extensions.
#[derive(Debug, Clone)]
pub struct HostCtx {
    /// `None` = the base host.
    pub space: Option<space::Model>,
    /// The request's own origin (CSRF).
    pub origin: String,
}

impl HostCtx {
    pub fn space_id(&self) -> Option<SpaceId> {
        self.space.as_ref().map(|s| SpaceId::from_row(s.id))
    }

    /// The host's space requires TOTP for new logins / credentials.
    pub fn requires_mfa(&self) -> bool {
        self.space.as_ref().is_some_and(|s| s.require_mfa)
    }
}

/// Every hosted `/api` route: unknown host or unknown slug → 404.
pub async fn classify(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let host = request_host(req.uri(), req.headers()).ok_or(AppError::NotFound)?;
    let space = match state.public.classify(&host) {
        HostKind::Base => None,
        HostKind::Slug(slug) => Some(
            repo::find_by_slug(&state.db, &slug)
                .await?
                .ok_or(AppError::NotFound)?,
        ),
        HostKind::Unknown => return Err(AppError::NotFound),
    };
    let origin = state.public.origin_of(&host);
    req.extensions_mut().insert(HostCtx { space, origin });
    Ok(next.run(req).await)
}

/// Space-host-only routes: on the base host they do not exist.
pub async fn space_only(req: Request, next: Next) -> Result<Response, AppError> {
    match req.extensions().get::<HostCtx>() {
        Some(HostCtx { space: Some(_), .. }) => Ok(next.run(req).await),
        _ => Err(AppError::NotFound),
    }
}

/// Base-host-only routes.
pub async fn base_only(req: Request, next: Next) -> Result<Response, AppError> {
    match req.extensions().get::<HostCtx>() {
        Some(HostCtx { space: None, .. }) => Ok(next.run(req).await),
        _ => Err(AppError::NotFound),
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContextKind {
    Base,
    Space,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContextSpace {
    pub slug: String,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub kind: ContextKind,
    pub space: Option<ContextSpace>,
    pub registration: bool,
    pub base_url: String,
    /// Sign in with Google is configured.
    pub google: bool,
}

/// Which app the SPA shows on this host.
#[utoipa::path(
    get,
    path = "/api/context",
    tag = "auth",
    responses(
        (status = 200, body = Context),
        (status = 404, description = "Unknown host", body = ErrorBody),
    )
)]
pub async fn context(
    State(state): State<AppState>,
    axum::Extension(host): axum::Extension<HostCtx>,
) -> Json<Context> {
    Json(Context {
        kind: if host.space.is_some() {
            ContextKind::Space
        } else {
            ContextKind::Base
        },
        space: host.space.map(|s| ContextSpace {
            slug: s.slug,
            name: s.name,
        }),
        registration: state.registration,
        base_url: state.public.base_url(),
        google: state.google.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> PublicUrl {
        PublicUrl::parse(s).expect("valid")
    }

    #[test]
    fn parses_public_url() {
        assert_eq!(
            url("https://InvoiceApp.cz/"),
            PublicUrl {
                https: true,
                host: "invoiceapp.cz".into(),
                port: None
            }
        );
        let dev = url("http://localhost:3000");
        assert_eq!((dev.https, dev.port), (false, Some(3000)));
        assert_eq!(dev.base_url(), "http://localhost:3000");
        assert_eq!(dev.space_url("firma"), "http://firma.localhost:3000");
        for bad in [
            "",
            "invoiceapp.cz",
            "ftp://x.cz",
            "https://x.cz/app",
            "https://x.cz:99999",
            "https://u@x.cz",
            "https://[::1]",
            "https://",
            "https://a..b",
        ] {
            assert!(PublicUrl::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn classifies_hosts() {
        let u = url("https://invoiceapp.cz");
        assert_eq!(u.classify("invoiceapp.cz"), HostKind::Base);
        assert_eq!(u.classify("InvoiceApp.CZ:443"), HostKind::Base);
        assert_eq!(u.classify("invoiceapp.cz."), HostKind::Base);
        assert_eq!(
            u.classify("firma.invoiceapp.cz"),
            HostKind::Slug("firma".into())
        );
        assert_eq!(
            u.classify("Firma.invoiceapp.cz:8443"),
            HostKind::Slug("firma".into())
        );
        assert_eq!(u.classify("a.firma.invoiceapp.cz"), HostKind::Unknown);
        // Slug shape only: a reserved name is looked up (and found only for a
        // space created before the name was reserved).
        assert_eq!(
            u.classify("www.invoiceapp.cz"),
            HostKind::Slug("www".into())
        );
        assert_eq!(u.classify("ab.invoiceapp.cz"), HostKind::Unknown);
        assert_eq!(u.classify("evilinvoiceapp.cz"), HostKind::Unknown);
        assert_eq!(u.classify("example.com"), HostKind::Unknown);
        assert_eq!(u.classify("[::1]:3000"), HostKind::Unknown);
        assert_eq!(u.classify(".invoiceapp.cz"), HostKind::Unknown);
    }

    #[test]
    fn origins() {
        let u = url("http://localhost:3000");
        assert_eq!(
            u.origin_of("Firma.localhost:3000"),
            "http://firma.localhost:3000"
        );
    }
}
