//! The downloaded design overrides of every space, by storage key: bounded
//! by their total size (`INVOICE__DESIGN_CACHE_MB`), least recently used
//! evicted first. Also each space's live listing, for [`LISTING_TTL`]: a
//! render does not list the storage again until it expires or the editor
//! invalidates the space (activation, restore, reset); a change made
//! outside the editor (`invoice design push`) shows within the TTL.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::error::AppError;
use crate::storage::{Object, Storage, Version};

/// `INVOICE__DESIGN_CACHE_MB` default.
pub const DEFAULT_LIMIT_MB: u32 = 128;

/// How long a space's live design listing is reused.
pub const LISTING_TTL: Duration = Duration::from_secs(30);

/// A live listing by design path.
pub type Listing = BTreeMap<String, Object>;

struct CachedListing {
    at: Instant,
    files: Listing,
}

struct Entry {
    version: Version,
    bytes: Bytes,
    /// The tick of the last hit or store.
    used: u64,
}

struct Inner {
    map: HashMap<String, Entry>,
    /// Sum of the cached files' lengths.
    total: u64,
    limit: u64,
    tick: u64,
    listings: HashMap<String, CachedListing>,
    listing_ttl: Duration,
    /// Bumped by every invalidation: a listing read before one is not
    /// stored after it.
    epoch: u64,
}

impl Inner {
    fn remove(&mut self, key: &str) {
        if let Some(e) = self.map.remove(key) {
            self.total -= e.bytes.len() as u64;
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }
}

/// Shared by every clone of the PDF service; entries are namespaced by the
/// space prefix (`{scope}|{key}`).
#[derive(Clone)]
pub struct Cache(Arc<Mutex<Inner>>);

impl Default for Cache {
    fn default() -> Self {
        Self::with_limit(u64::from(DEFAULT_LIMIT_MB) * 1024 * 1024)
    }
}

impl Cache {
    /// A cache holding at most `limit` bytes of file contents.
    pub fn with_limit(limit: u64) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            map: HashMap::new(),
            total: 0,
            limit,
            tick: 0,
            listings: HashMap::new(),
            listing_ttl: LISTING_TTL,
            epoch: 0,
        })))
    }

    /// Reuse a listing for `ttl` instead of [`LISTING_TTL`] (`ZERO`: list
    /// on every render).
    pub fn with_listing_ttl(self, ttl: Duration) -> Self {
        self.inner().listing_ttl = ttl;
        self
    }

    /// The fresh cached listing of `scope`, else the epoch to hand to
    /// [`store_listing`](Self::store_listing) with the new one.
    pub(super) fn listing(&self, scope: &str) -> Result<Listing, u64> {
        let inner = self.inner();
        match inner.listings.get(scope) {
            Some(l) if l.at.elapsed() < inner.listing_ttl => Ok(l.files.clone()),
            _ => Err(inner.epoch),
        }
    }

    /// Drop the listing of `scope` (it names a file that is gone); the
    /// epoch for storing the next one.
    pub(super) fn forget_listing(&self, scope: &str) -> u64 {
        let mut inner = self.inner();
        inner.listings.remove(scope);
        inner.epoch
    }

    /// Keep `files` as the listing of `scope`, unless an invalidation came
    /// after `epoch` (the listing may predate it). Expired listings of
    /// other spaces are dropped here.
    pub(super) fn store_listing(&self, scope: &str, epoch: u64, files: Listing) {
        let mut inner = self.inner();
        if inner.epoch != epoch || inner.listing_ttl.is_zero() {
            return;
        }
        let ttl = inner.listing_ttl;
        inner.listings.retain(|_, l| l.at.elapsed() < ttl);
        inner.listings.insert(
            scope.to_string(),
            CachedListing {
                at: Instant::now(),
                files,
            },
        );
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Cached files (tests check what was downloaded).
    pub fn len(&self) -> usize {
        self.inner().map.len()
    }

    /// Cached bytes.
    pub fn total_bytes(&self) -> u64 {
        self.inner().total
    }

    fn hit(&self, key: &str, version: &Version) -> Option<Bytes> {
        let mut inner = self.inner();
        let tick = inner.next_tick();
        let e = inner.map.get_mut(key).filter(|e| &e.version == version)?;
        e.used = tick;
        Some(e.bytes.clone())
    }

    /// Store `bytes` under `key`, then evict the least recently used
    /// entries until the total fits. A file larger than the whole limit is
    /// not cached.
    fn store(&self, key: String, version: Version, bytes: Bytes) {
        let mut inner = self.inner();
        inner.remove(&key);
        let size = bytes.len() as u64;
        if size > inner.limit {
            return;
        }
        let used = inner.next_tick();
        inner.total += size;
        inner.map.insert(
            key,
            Entry {
                version,
                bytes,
                used,
            },
        );
        // Linear per eviction: a design has tens of files, the cache a few
        // hundred entries.
        while inner.total > inner.limit {
            let Some(oldest) = inner
                .map
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            inner.remove(&oldest);
        }
    }

    /// Drop the entries of `scope` whose key is not in `keys` (other
    /// scopes untouched).
    pub(super) fn retain(&self, scope: &str, keys: &[String]) {
        let prefix = format!("{scope}|");
        let mut inner = self.inner();
        let gone: Vec<String> = inner
            .map
            .keys()
            .filter(|k| k.starts_with(&prefix) && !keys.contains(k))
            .cloned()
            .collect();
        for k in gone {
            inner.remove(&k);
        }
    }

    /// Forget every cached file and the listing of `scope` (a design
    /// activation, restore or reset of that space, or the space's deletion).
    pub fn invalidate(&self, scope: &str) {
        self.retain(scope, &[]);
        let mut inner = self.inner();
        inner.listings.remove(scope);
        inner.epoch += 1;
    }

    pub(super) async fn fetch(
        &self,
        storage: &Storage,
        scope: &str,
        o: &Object,
    ) -> Result<Bytes, AppError> {
        let cache_key = scoped_key(scope, &o.key);
        if let Some(bytes) = self.hit(&cache_key, &o.version) {
            return Ok(bytes);
        }
        // Cached under the listed version (a GET reports e.g. a coarser
        // Last-Modified): a file replaced between the listing and this read
        // no longer matches the next listing and is fetched again.
        let bytes = storage.get(&o.key).await?;
        self.store(cache_key, o.version.clone(), bytes.clone());
        Ok(bytes)
    }
}

pub(super) fn scoped_key(scope: &str, key: &str) -> String {
    format!("{scope}|{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(size: u64) -> Version {
        Version {
            e_tag: None,
            size,
            last_modified: chrono::DateTime::UNIX_EPOCH,
        }
    }

    fn put(c: &Cache, key: &str, size: usize) {
        c.store(key.into(), version(size as u64), Bytes::from(vec![0; size]));
    }

    fn has(c: &Cache, key: &str, size: usize) -> bool {
        c.hit(key, &version(size as u64)).is_some()
    }

    #[test]
    fn evicts_least_recently_used_by_bytes() {
        let c = Cache::with_limit(100);
        put(&c, "s|a", 40);
        put(&c, "s|b", 40);
        assert!(has(&c, "s|a", 40), "a is now the most recent");
        put(&c, "s|c", 40);
        assert!(!has(&c, "s|b", 40), "b was the least recently used");
        assert!(has(&c, "s|a", 40) && has(&c, "s|c", 40));
        assert_eq!((c.len(), c.total_bytes()), (2, 80));
    }

    #[test]
    fn replacing_an_entry_counts_once() {
        let c = Cache::with_limit(100);
        put(&c, "s|a", 60);
        put(&c, "s|a", 30);
        assert_eq!((c.len(), c.total_bytes()), (1, 30));
        assert!(!has(&c, "s|a", 60), "the old version is gone");
    }

    #[test]
    fn oversized_file_is_not_cached() {
        let c = Cache::with_limit(100);
        put(&c, "s|a", 50);
        put(&c, "s|big", 101);
        assert_eq!((c.len(), c.total_bytes()), (1, 50));
    }

    fn listed(c: &Cache, scope: &str) -> Option<usize> {
        c.listing(scope).ok().map(|l| l.len())
    }

    fn object(key: &str) -> Object {
        Object {
            key: key.into(),
            version: version(1),
        }
    }

    #[test]
    fn listing_is_reused_until_invalidated() {
        let c = Cache::with_limit(100);
        let epoch = c.listing("s1").expect_err("nothing cached");
        let files = Listing::from([("a".to_string(), object("design/a"))]);
        c.store_listing("s1", epoch, files.clone());
        assert_eq!(listed(&c, "s1"), Some(1));
        assert_eq!(listed(&c, "s2"), None, "per scope");
        c.invalidate("s2");
        assert_eq!(listed(&c, "s1"), Some(1), "other scopes keep theirs");
        c.invalidate("s1");
        assert_eq!(listed(&c, "s1"), None);
    }

    #[test]
    fn a_listing_older_than_an_invalidation_is_not_stored() {
        let c = Cache::with_limit(100);
        let epoch = c.listing("s1").expect_err("nothing cached");
        c.invalidate("s1");
        c.store_listing("s1", epoch, Listing::new());
        assert_eq!(listed(&c, "s1"), None);
    }

    #[test]
    fn a_zero_ttl_never_reuses() {
        let c = Cache::with_limit(100).with_listing_ttl(Duration::ZERO);
        let epoch = c.listing("s1").expect_err("nothing cached");
        c.store_listing("s1", epoch, Listing::new());
        assert_eq!(listed(&c, "s1"), None);
    }

    #[test]
    fn invalidate_and_retain_are_per_scope() {
        let c = Cache::with_limit(1000);
        put(&c, "s1|a", 10);
        put(&c, "s1|b", 10);
        put(&c, "s2|a", 10);
        c.retain("s1", &["s1|b".to_string()]);
        assert_eq!((c.len(), c.total_bytes()), (2, 20));
        c.invalidate("s2");
        assert!(has(&c, "s1|b", 10) && !has(&c, "s2|a", 10));
        assert_eq!(c.total_bytes(), 10);
    }
}
