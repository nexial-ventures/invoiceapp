//! The client address rate limits are keyed by (`INVOICEAPP__TRUST_FORWARDED`,
//! `docs/api/auth.md`).

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;

use crate::app::AppState;

/// The rate-limit key of the client: with a trusted reverse proxy the
/// **last** `X-Forwarded-For` hop (the one our proxy appended — earlier hops
/// are client-controlled), else the socket address; `"unknown"` when neither
/// exists (in-process test requests). See [`rate_key`].
pub fn client_ip(parts: &Parts, trust_forwarded: bool) -> String {
    let forwarded = trust_forwarded
        .then(|| forwarded_ip(&parts.headers))
        .flatten();
    forwarded
        .or_else(|| {
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip())
        })
        .map_or_else(|| "unknown".to_string(), rate_key)
}

/// IPv4 as is; IPv6 by its /64 — one subscriber usually holds a whole /64,
/// so per-address keys would let a single client rotate through 2^64
/// buckets. An IPv4-mapped IPv6 address counts as the IPv4 one.
pub fn rate_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            let net = Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0);
            format!("{net}/64")
        }
    }
}

/// The last hop of the last `X-Forwarded-For` header. A header that is
/// present but whose last hop does not parse logs one warning (the limits
/// then fall back to the socket address — the proxy's).
fn forwarded_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let value = headers.get_all("x-forwarded-for").iter().next_back()?;
    let ip = value
        .to_str()
        .ok()
        .and_then(|v| v.rsplit(',').next())
        .and_then(|hop| parse_hop(hop.trim()));
    if ip.is_none() && !UNPARSABLE_WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "X-Forwarded-For has an unparsable last hop: rate limits use the socket address \
             (check what the reverse proxy appends)"
        );
    }
    ip
}

/// A bare address, or one with a port (`203.0.113.7:51234`,
/// `[2001:db8::1]:443`) as some proxies append it.
fn parse_hop(hop: &str) -> Option<IpAddr> {
    hop.parse::<IpAddr>()
        .ok()
        .or_else(|| hop.parse::<SocketAddr>().ok().map(|s| s.ip()))
}

static UNPARSABLE_WARNED: AtomicBool = AtomicBool::new(false);

static WARNED: AtomicBool = AtomicBool::new(false);

/// Warn once per process when requests carry `X-Forwarded-For` while it is
/// not trusted: behind a proxy every client then shares the proxy's address
/// (and its rate-limit buckets).
pub async fn warn_untrusted_forwarded(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if !state.trust_forwarded
        && req.headers().contains_key("x-forwarded-for")
        && !WARNED.swap(true, Ordering::Relaxed)
    {
        tracing::warn!(
            "requests carry X-Forwarded-For but INVOICEAPP__TRUST_FORWARDED is false: rate limits \
             key every client by the proxy's address; set it to true behind a reverse proxy \
             (and keep the app port unreachable directly)"
        );
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_for_last_hop() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "6.6.6.6, 203.0.113.7".parse().unwrap());
        assert_eq!(forwarded_ip(&h), "203.0.113.7".parse().ok());
        h.insert("x-forwarded-for", "garbage".parse().unwrap());
        assert_eq!(forwarded_ip(&h), None);
    }

    #[test]
    fn hops_with_a_port_parse() {
        let ip = |s: &str| s.parse::<IpAddr>().ok();
        assert_eq!(parse_hop("203.0.113.7"), ip("203.0.113.7"));
        assert_eq!(parse_hop("203.0.113.7:51234"), ip("203.0.113.7"));
        assert_eq!(parse_hop("2001:db8::1"), ip("2001:db8::1"));
        assert_eq!(parse_hop("[2001:db8::1]:443"), ip("2001:db8::1"));
        assert_eq!(parse_hop("unknown"), None);
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            "6.6.6.6, 203.0.113.7:51234".parse().unwrap(),
        );
        assert_eq!(forwarded_ip(&h), ip("203.0.113.7"));
        h.insert("x-forwarded-for", "[2001:db8::1]:443".parse().unwrap());
        assert_eq!(forwarded_ip(&h), ip("2001:db8::1"));
    }

    #[test]
    fn ipv6_is_keyed_by_its_64() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(rate_key(ip("203.0.113.7")), "203.0.113.7");
        assert_eq!(
            rate_key(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd")),
            "2001:db8:1:2::/64"
        );
        assert_eq!(
            rate_key(ip("2001:db8:1:2::1")),
            rate_key(ip("2001:db8:1:2:ffff::9"))
        );
        assert_ne!(
            rate_key(ip("2001:db8:1:2::1")),
            rate_key(ip("2001:db8:1:3::1"))
        );
        assert_eq!(rate_key(ip("::ffff:198.51.100.4")), "198.51.100.4");
    }

    #[test]
    fn client_ip_sources() {
        let parts = |xff: Option<&str>, sock: Option<&str>| {
            let mut b = axum::http::Request::builder();
            if let Some(x) = xff {
                b = b.header("x-forwarded-for", x);
            }
            let (mut p, _) = b.body(()).unwrap().into_parts();
            if let Some(s) = sock {
                p.extensions
                    .insert(ConnectInfo(s.parse::<SocketAddr>().unwrap()));
            }
            p
        };
        let p = parts(Some("2001:db8::5"), Some("[2001:db8:9:9::1]:4000"));
        assert_eq!(client_ip(&p, true), "2001:db8::/64");
        assert_eq!(client_ip(&p, false), "2001:db8:9:9::/64");
        assert_eq!(client_ip(&parts(Some("junk"), None), true), "unknown");
        assert_eq!(client_ip(&parts(None, None), false), "unknown");
    }
}
