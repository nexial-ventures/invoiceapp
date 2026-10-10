//! The provider's signing keys, cached. One fetch at a time: concurrent
//! callers on a cold or expired cache wait for that fetch instead of
//! failing. The set expires after the response's `Cache-Control: max-age`
//! (default 1 h, at most 24 h); an unknown `kid` refetches it, at most once
//! per gate interval (60 s).

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use tokio::sync::Mutex;

use super::Fail;

pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(60 * 60);
pub const MAX_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The lifetime a `Cache-Control` header grants: `max-age`, capped at
/// [`MAX_MAX_AGE`]; missing / unparsable → [`DEFAULT_MAX_AGE`]; `no-store`
/// / `no-cache` → 0 (refetched on next use).
pub fn max_age(cache_control: Option<&str>) -> Duration {
    let Some(cc) = cache_control else {
        return DEFAULT_MAX_AGE;
    };
    let mut age = None;
    for part in cc.split(',').map(|p| p.trim().to_ascii_lowercase()) {
        if part == "no-store" || part == "no-cache" {
            return Duration::ZERO;
        }
        if let Some(v) = part.strip_prefix("max-age=") {
            age = v.trim_matches('"').parse::<u64>().ok();
        }
    }
    age.map_or(DEFAULT_MAX_AGE, |s| Duration::from_secs(s).min(MAX_MAX_AGE))
}

struct Keys {
    set: Arc<JwkSet>,
    fetched: Instant,
    ttl: Duration,
}

pub struct JwksCache {
    /// The shortest interval between two fetches for an unknown `kid`.
    gate: Duration,
    keys: Mutex<Option<Keys>>,
}

impl JwksCache {
    pub fn new(gate: Duration) -> Self {
        Self {
            gate,
            keys: Mutex::new(None),
        }
    }

    async fn store<F, Fut>(slot: &mut Option<Keys>, fetch: F) -> Result<Arc<JwkSet>, Fail>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(JwkSet, Duration), Fail>>,
    {
        let (set, ttl) = fetch().await?;
        let set = Arc::new(set);
        *slot = Some(Keys {
            set: set.clone(),
            fetched: Instant::now(),
            ttl,
        });
        Ok(set)
    }

    /// The live key set: cached and unexpired, else fetched now. A failed
    /// fetch is `unavailable`.
    pub async fn current<F, Fut>(&self, fetch: F) -> Result<Arc<JwkSet>, Fail>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(JwkSet, Duration), Fail>>,
    {
        let mut slot = self.keys.lock().await;
        match &*slot {
            Some(k) if k.fetched.elapsed() < k.ttl => Ok(k.set.clone()),
            _ => Self::store(&mut slot, fetch).await,
        }
    }

    /// `seen` lacked the token's `kid`: a newer set — the one another
    /// caller fetched meanwhile, or a fresh fetch when the last one is at
    /// least the gate interval old. `None` = nothing newer may be fetched
    /// yet.
    pub async fn newer<F, Fut>(
        &self,
        seen: &Arc<JwkSet>,
        fetch: F,
    ) -> Result<Option<Arc<JwkSet>>, Fail>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(JwkSet, Duration), Fail>>,
    {
        let mut slot = self.keys.lock().await;
        match &*slot {
            Some(k) if !Arc::ptr_eq(&k.set, seen) => Ok(Some(k.set.clone())),
            Some(k) if k.fetched.elapsed() < self.gate => Ok(None),
            _ => Self::store(&mut slot, fetch).await.map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn cache_control() {
        assert_eq!(max_age(None), DEFAULT_MAX_AGE);
        assert_eq!(
            max_age(Some("public, max-age=19800, must-revalidate")),
            Duration::from_secs(19800)
        );
        assert_eq!(max_age(Some("max-age=999999999")), MAX_MAX_AGE);
        assert_eq!(max_age(Some("max-age=nope")), DEFAULT_MAX_AGE);
        assert_eq!(max_age(Some("private")), DEFAULT_MAX_AGE);
        assert_eq!(max_age(Some("no-store")), Duration::ZERO);
        assert_eq!(max_age(Some("Max-Age=60")), Duration::from_secs(60));
    }

    fn empty() -> JwkSet {
        JwkSet { keys: Vec::new() }
    }

    #[tokio::test]
    async fn cold_cache_is_fetched_once_for_concurrent_callers() {
        let cache = Arc::new(JwksCache::new(Duration::from_secs(60)));
        let fetches = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (cache, fetches) = (cache.clone(), fetches.clone());
            tasks.push(tokio::spawn(async move {
                cache
                    .current(|| async {
                        fetches.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Ok((empty(), DEFAULT_MAX_AGE))
                    })
                    .await
            }));
        }
        for t in tasks {
            assert!(t.await.expect("task").is_ok());
        }
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failures_expiry_and_the_refresh_gate() {
        let cache = JwksCache::new(Duration::from_secs(60));
        let failed = cache.current(|| async { Err(Fail::Unavailable) }).await;
        assert_eq!(failed.err(), Some(Fail::Unavailable));
        // Expired at once (max-age 0): the next use fetches again.
        let a = cache
            .current(|| async { Ok((empty(), Duration::ZERO)) })
            .await
            .expect("fetched");
        let b = cache
            .current(|| async { Ok((empty(), DEFAULT_MAX_AGE)) })
            .await
            .expect("refetched");
        assert!(!Arc::ptr_eq(&a, &b));
        // Unknown kid in `a` (stale): the newer `b` without a fetch.
        let n = cache
            .newer(&a, || async { Err(Fail::Unavailable) })
            .await
            .expect("no fetch");
        assert!(n.is_some_and(|n| Arc::ptr_eq(&n, &b)));
        // Unknown kid in the newest set, fetched just now: gated.
        let n = cache.newer(&b, || async { Err(Fail::Unavailable) }).await;
        assert_eq!(n.ok().flatten().map(|_| ()), None);
        // Gate passed: fetched (failure → unavailable).
        let open = JwksCache::new(Duration::ZERO);
        let c = open
            .current(|| async { Ok((empty(), DEFAULT_MAX_AGE)) })
            .await
            .expect("fetched");
        let n = open.newer(&c, || async { Err(Fail::Unavailable) }).await;
        assert_eq!(n.err(), Some(Fail::Unavailable));
    }
}
