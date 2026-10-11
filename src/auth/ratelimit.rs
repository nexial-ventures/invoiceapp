//! In-memory sliding-window rate limits (per instance): login failures,
//! password checks, the e-mail-sending auth routes and design activations.
//! 429 `rate_limited` + `Retry-After`.
//!
//! Attempts are **reserved** before the slow work (argon2, DB) — check and
//! record of every bucket under one lock — so a concurrent burst cannot slip
//! past the limit; a caller that only counts failures refunds on success.
//! Keys are attacker-chosen (e-mails): the map is swept in insertion order a
//! few entries per call and hard-capped, so it stays bounded at O(1)
//! amortized cost per request.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use axum::http::request::Parts;

use crate::error::AppError;

const FIFTEEN_MIN: Duration = Duration::from_secs(15 * 60);
const HOUR: Duration = Duration::from_secs(60 * 60);
/// The longest window of any limit: older events never matter.
const MAX_WINDOW: Duration = HOUR;
/// Most keys tracked; beyond it the oldest-inserted keys are dropped.
const MAX_KEYS: usize = 100_000;
/// Keys examined by the sweep per reservation.
const SWEEP: usize = 4;
/// Keys are cut to this many bytes (an e-mail is at most 254).
const MAX_KEY: usize = 320;

/// One limit: at most `max` events per `window`.
#[derive(Debug, Clone, Copy)]
pub struct Limit {
    pub name: &'static str,
    pub max: usize,
    pub window: Duration,
}

/// Failed logins per e-mail; also failed password checks of a signed-in user
/// (password change, space delete), keyed by the user's e-mail.
pub const LOGIN_EMAIL: Limit = Limit {
    name: "login-email",
    max: 5,
    window: FIFTEEN_MIN,
};
pub const LOGIN_IP: Limit = Limit {
    name: "login-ip",
    max: 20,
    window: FIFTEEN_MIN,
};
/// Register, verify/resend and password-reset share these two buckets.
pub const MAIL_IP: Limit = Limit {
    name: "mail-ip",
    max: 5,
    window: HOUR,
};
pub const MAIL_EMAIL: Limit = Limit {
    name: "mail-email",
    max: 3,
    window: HOUR,
};
/// Password-reset confirmations per IP (every request counts).
pub const RESET_IP: Limit = Limit {
    name: "reset-ip",
    max: 20,
    window: FIFTEEN_MIN,
};

/// Invitation lookups and accepts per IP (every request counts).
pub const INVITE_IP: Limit = Limit {
    name: "invite-ip",
    max: 20,
    window: FIFTEEN_MIN,
};

/// Google sign-in starts per IP (every request counts).
pub const GOOGLE_IP: Limit = Limit {
    name: "google-ip",
    max: 20,
    window: FIFTEEN_MIN,
};

/// Invitation e-mails (create + resend) per inviting user and per space.
pub const INVITE_USER: Limit = Limit {
    name: "invite-user",
    max: 20,
    window: HOUR,
};
pub const INVITE_SPACE: Limit = Limit {
    name: "invite-space",
    max: 50,
    window: HOUR,
};

/// Design activations and restores per space (each costs two renders).
pub const DESIGN_SPACE: Limit = Limit {
    name: "design-space",
    max: 10,
    window: HOUR,
};

type Key = (&'static str, String);

#[derive(Default)]
struct Inner {
    events: HashMap<Key, VecDeque<Instant>>,
    /// Keys in insertion order (the sweep / eviction queue).
    order: VecDeque<Key>,
}

impl Inner {
    fn key(limit: Limit, key: &str) -> Key {
        let mut end = key.len().min(MAX_KEY);
        while !key.is_char_boundary(end) {
            end -= 1;
        }
        (limit.name, key[..end].to_string())
    }

    /// Drop a few stale keys from the front of the queue.
    fn sweep(&mut self, now: Instant) {
        for _ in 0..SWEEP {
            let Some(front) = self.order.front() else {
                break;
            };
            let stale = self.events.get(front).is_none_or(|ev| {
                ev.back()
                    .is_none_or(|t| now.duration_since(*t) >= MAX_WINDOW)
            });
            let Some(key) = self.order.pop_front() else {
                break;
            };
            if stale {
                self.events.remove(&key);
            } else {
                // Still active: look at it again after the others.
                self.order.push_back(key);
            }
        }
    }

    /// Drop the oldest-inserted keys while over [`MAX_KEYS`].
    fn cap(&mut self) {
        while self.events.len() > MAX_KEYS {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            self.events.remove(&key);
        }
    }

    fn wait(&mut self, limit: Limit, key: &Key, now: Instant) -> Option<Duration> {
        let events = self.events.get_mut(key)?;
        prune(events, limit.window, now);
        (events.len() >= limit.max).then(|| {
            let oldest = events.front().copied().unwrap_or(now);
            (oldest + limit.window).saturating_duration_since(now)
        })
    }
}

#[derive(Clone, Default)]
pub struct RateLimiter(Arc<Mutex<Inner>>);

impl RateLimiter {
    /// Reserve one attempt in every bucket at once, or none: 429 when any is
    /// used up.
    pub fn reserve(&self, buckets: &[(Limit, &str)]) -> Result<(), AppError> {
        self.reserve_at(buckets, Instant::now())
    }

    /// Give back the attempts of an earlier [`reserve`](Self::reserve) (a
    /// success where only failures count).
    pub fn refund(&self, buckets: &[(Limit, &str)]) {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for (limit, key) in buckets {
            if let Some(ev) = inner.events.get_mut(&Inner::key(*limit, key)) {
                ev.pop_back();
            }
        }
    }

    fn reserve_at(&self, buckets: &[(Limit, &str)], now: Instant) -> Result<(), AppError> {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.sweep(now);
        let keys: Vec<(Limit, Key)> = buckets
            .iter()
            .map(|(l, k)| (*l, Inner::key(*l, k)))
            .collect();
        let wait = keys
            .iter()
            .filter_map(|(l, k)| inner.wait(*l, k, now))
            .max();
        if let Some(w) = wait {
            return Err(AppError::RateLimited(w.as_secs().max(1)));
        }
        for (_, key) in keys {
            if !inner.events.contains_key(&key) {
                inner.order.push_back(key.clone());
            }
            inner.events.entry(key).or_default().push_back(now);
        }
        inner.cap();
        Ok(())
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .events
            .len()
    }
}

fn prune(events: &mut VecDeque<Instant>, window: Duration, now: Instant) {
    while events
        .front()
        .is_some_and(|t| now.duration_since(*t) >= window)
    {
        events.pop_front();
    }
}

/// The client IP: with a trusted reverse proxy the **last** `X-Forwarded-For`
/// hop (the one our proxy appended — earlier hops are client-controlled),
/// else the socket address; `"unknown"` when neither exists (in-process
/// test requests).
pub fn client_ip(parts: &Parts, trust_forwarded: bool) -> String {
    if trust_forwarded && let Some(ip) = forwarded_ip(&parts.headers) {
        return ip.to_string();
    }
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn forwarded_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .next_back()?
        .to_str()
        .ok()?
        .rsplit(',')
        .next()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIM: Limit = Limit {
        name: "t",
        max: 2,
        window: Duration::from_secs(60),
    };

    #[test]
    fn sliding_window_and_refund() {
        let rl = RateLimiter::default();
        let t0 = Instant::now();
        assert!(rl.reserve_at(&[(LIM, "k")], t0).is_ok());
        assert!(
            rl.reserve_at(&[(LIM, "k")], t0 + Duration::from_secs(10))
                .is_ok()
        );
        match rl.reserve_at(&[(LIM, "k")], t0 + Duration::from_secs(20)) {
            Err(AppError::RateLimited(s)) => assert_eq!(s, 40),
            other => panic!("expected 429, got {other:?}"),
        }
        assert!(rl.reserve_at(&[(LIM, "other")], t0).is_ok());
        rl.refund(&[(LIM, "k")]);
        assert!(
            rl.reserve_at(&[(LIM, "k")], t0 + Duration::from_secs(20))
                .is_ok()
        );
        assert!(
            rl.reserve_at(&[(LIM, "k")], t0 + Duration::from_secs(70))
                .is_ok()
        );
    }

    #[test]
    fn all_or_nothing() {
        let rl = RateLimiter::default();
        let t0 = Instant::now();
        rl.reserve_at(&[(LIM, "a")], t0).expect("1");
        rl.reserve_at(&[(LIM, "a")], t0).expect("2");
        assert!(rl.reserve_at(&[(LIM, "b"), (LIM, "a")], t0).is_err());
        // "b" was not charged by the refused reservation.
        rl.reserve_at(&[(LIM, "b")], t0).expect("b 1");
        rl.reserve_at(&[(LIM, "b")], t0).expect("b 2");
    }

    #[test]
    fn stale_keys_are_swept_and_the_map_is_capped() {
        let rl = RateLimiter::default();
        let t0 = Instant::now();
        for i in 0..10 {
            rl.reserve_at(&[(LIM, &format!("k{i}"))], t0).expect("ok");
        }
        assert_eq!(rl.len(), 10);
        let later = t0 + MAX_WINDOW;
        for _ in 0..3 {
            rl.reserve_at(&[(LIM, "fresh")], later).ok();
        }
        assert!(rl.len() <= 10 - 2 * SWEEP + 1, "{}", rl.len());
        let rl = RateLimiter::default();
        for i in 0..MAX_KEYS + 5 {
            rl.reserve_at(&[(LIM, &i.to_string())], t0).expect("ok");
        }
        assert!(rl.len() <= MAX_KEYS);
    }

    #[test]
    fn long_keys_are_cut_on_a_char_boundary() {
        let k = Inner::key(LIM, &"ž".repeat(400));
        assert!(k.1.len() <= MAX_KEY);
    }

    #[test]
    fn forwarded_for_last_hop() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "6.6.6.6, 203.0.113.7".parse().unwrap());
        assert_eq!(forwarded_ip(&h), "203.0.113.7".parse().ok());
        h.insert("x-forwarded-for", "garbage".parse().unwrap());
        assert_eq!(forwarded_ip(&h), None);
    }
}
