//! In-memory rate limits (per instance): login failures (a progressive delay
//! per e-mail, hard per IP), code and password checks, the e-mail-sending
//! auth routes and design activations. 429 `rate_limited` + `Retry-After`.
//!
//! Attempts are **reserved** before the slow work (argon2, DB) — check and
//! record of every bucket under one lock — so a concurrent burst cannot slip
//! past the limit; a caller that only counts failures refunds on success.
//! Keys are attacker-chosen (e-mails): the map is swept in insertion order a
//! few entries per call, dropping only **idle** keys (nothing left in their
//! window or delay — lossless), and hard-capped as a last resort, so it stays
//! bounded at O(1) amortized cost per request without an attacker being able
//! to flush the state of a key under attack.

mod limits;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use limits::*;

use crate::error::AppError;

/// Most keys tracked; beyond it idle keys go first, then the oldest.
const MAX_KEYS: usize = 100_000;
/// Keys examined by the sweep per reservation.
const SWEEP: usize = 4;
/// Keys are cut to this many bytes (an e-mail is at most 254).
const MAX_KEY: usize = 320;

type Key = (&'static str, String);

struct Entry {
    /// [`Limit::span`] of the key's limit.
    span: Duration,
    events: VecDeque<Instant>,
}

impl Entry {
    fn idle(&self, now: Instant) -> bool {
        self.events
            .back()
            .is_none_or(|t| now.duration_since(*t) >= self.span)
    }
}

struct Inner {
    entries: HashMap<Key, Entry>,
    /// Keys in insertion order (the sweep / eviction queue).
    order: VecDeque<Key>,
    max_keys: usize,
    /// The last-resort eviction has been logged.
    warned: bool,
}

impl Inner {
    fn new(max_keys: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            max_keys,
            warned: false,
        }
    }

    fn key(limit: Limit, key: &str) -> Key {
        let mut end = key.len().min(MAX_KEY);
        while !key.is_char_boundary(end) {
            end -= 1;
        }
        (limit.name, key[..end].to_string())
    }

    /// Drop a few idle keys from the front of the queue.
    fn sweep(&mut self, now: Instant) {
        for _ in 0..SWEEP {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            if self.entries.get(&key).is_none_or(|e| e.idle(now)) {
                self.entries.remove(&key);
            } else {
                // Still active: look at it again after the others.
                self.order.push_back(key);
            }
        }
    }

    /// While over `max_keys`: drop every idle key (they hold nothing that
    /// still counts), then — only if that is not enough — the oldest active.
    fn cap(&mut self, now: Instant) {
        if self.entries.len() <= self.max_keys {
            return;
        }
        self.entries.retain(|_, e| !e.idle(now));
        let entries = &self.entries;
        self.order.retain(|k| entries.contains_key(k));
        if self.entries.len() > self.max_keys && !self.warned {
            self.warned = true;
            tracing::warn!(
                keys = self.entries.len(),
                "rate limiter full of active keys: evicting the oldest (logged once)"
            );
        }
        while self.entries.len() > self.max_keys {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&key);
        }
    }

    fn wait(&mut self, limit: Limit, key: &Key, now: Instant) -> Option<Duration> {
        let entry = self.entries.get_mut(key)?;
        prune(&mut entry.events, limit.window, now);
        limit.wait(&entry.events, now)
    }
}

#[derive(Clone)]
pub struct RateLimiter(Arc<Mutex<Inner>>);

impl Default for RateLimiter {
    fn default() -> Self {
        Self::with_cap(MAX_KEYS)
    }
}

impl RateLimiter {
    fn with_cap(max_keys: usize) -> Self {
        Self(Arc::new(Mutex::new(Inner::new(max_keys))))
    }

    /// Reserve one attempt in every bucket at once, or none: 429 when any is
    /// used up or still in its delay.
    pub fn reserve(&self, buckets: &[(Limit, &str)]) -> Result<(), AppError> {
        self.reserve_at(buckets, Instant::now())
    }

    /// Give back the attempts of an earlier [`reserve`](Self::reserve) (a
    /// success where only failures count). Earlier failures stay: a correct
    /// password must not wipe the TOTP or login failures before it.
    pub fn refund(&self, buckets: &[(Limit, &str)]) {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for (limit, key) in buckets {
            if let Some(e) = inner.entries.get_mut(&Inner::key(*limit, key)) {
                e.events.pop_back();
            }
        }
    }

    /// Pass `result` through; an `Err(Busy)` (the argon2 gate was full, the
    /// password was never checked) first gives the reservation back — an
    /// attempt that did not happen must not count.
    pub fn refund_if_busy<T>(
        &self,
        buckets: &[(Limit, &str)],
        result: Result<T, AppError>,
    ) -> Result<T, AppError> {
        if matches!(result, Err(AppError::Busy)) {
            self.refund(buckets);
        }
        result
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
            return Err(AppError::RateLimited(retry_after(w)));
        }
        for (limit, key) in keys {
            if !inner.entries.contains_key(&key) {
                inner.order.push_back(key.clone());
            }
            let entry = inner.entries.entry(key).or_insert_with(|| Entry {
                span: limit.span(),
                events: VecDeque::new(),
            });
            entry.events.push_back(now);
        }
        inner.cap(now);
        Ok(())
    }
}

/// Whole seconds, rounded up (a client retrying after it must not be too
/// early), at least 1.
fn retry_after(wait: Duration) -> u64 {
    (wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1)
}

fn prune(events: &mut VecDeque<Instant>, window: Duration, now: Instant) {
    while events
        .front()
        .is_some_and(|t| now.duration_since(*t) >= window)
    {
        events.pop_front();
    }
}
