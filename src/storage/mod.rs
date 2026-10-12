//! File storage for every blob the app keeps: archived PDFs, uploaded
//! originals and design overrides. One [`Storage`] over the `object_store`
//! crate, backed by a local directory (`fs`) or an S3-compatible bucket (`s3`).
//!
//! Keys are `/`-separated (`documents/{year}/{id}.pdf`, `design/logo.svg`).

pub mod config;
pub mod local;
pub mod transfer;

use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::stream::BoxStream;
use futures_util::{StreamExt as _, TryStreamExt as _};
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::prefix::PrefixStore;
use object_store::{ClientOptions, ObjectStore, ObjectStoreExt as _, RetryConfig};
use sha2::{Digest, Sha256};

use crate::error::AppError;
pub use config::{S3Config, StorageConfig};

/// Design overrides live under `design/…`.
pub const DESIGN_PREFIX: &str = "design";

/// Bounds how long a request waits on an unreachable bucket before it fails
/// with `storage_unavailable` (the crate's default retries for minutes).
/// No total timeout: it would also cut off a slow but progressing download
/// stream; the read timeout bounds each wait for headers or the next chunk.
const S3_READ_TIMEOUT: Duration = Duration::from_secs(30);
const S3_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const S3_RETRY_TIMEOUT: Duration = Duration::from_secs(15);
const S3_MAX_RETRIES: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("object {0} not found")]
    NotFound(String),
    #[error("invalid storage key {0:?}")]
    InvalidKey(String),
    #[error("storage unavailable: {0}")]
    Unavailable(#[source] object_store::Error),
}

impl From<Error> for AppError {
    /// A missing object the database points at is an inconsistency (500);
    /// anything the backend fails with is `storage_unavailable` (503).
    fn from(err: Error) -> Self {
        match err {
            Error::Unavailable(e) => AppError::StorageUnavailable(e.to_string()),
            other => AppError::Internal(anyhow::Error::new(other)),
        }
    }
}

fn map_err(key: &str, err: object_store::Error) -> Error {
    match err {
        object_store::Error::NotFound { .. } => Error::NotFound(key.to_string()),
        e => Error::Unavailable(e),
    }
}

/// 8 random hex digits: makes a content-derived key unique per write
/// attempt, so a failed attempt only ever removes its own object.
pub fn nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What decides whether a cached copy is still current: the backend's ETag
/// plus size and modification time (a backend without ETags still changes
/// one of those on every write).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub e_tag: Option<String>,
    pub size: u64,
    pub last_modified: DateTime<Utc>,
}

/// A listed object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub key: String,
    pub version: Version,
}

impl Object {
    fn from_meta(meta: object_store::ObjectMeta) -> Self {
        Self {
            key: meta.location.to_string(),
            version: Version {
                e_tag: meta.e_tag,
                size: meta.size,
                last_modified: meta.last_modified,
            },
        }
    }
}

/// An object's body as a stream (downloads), with its size.
pub struct Download {
    pub size: u64,
    pub stream: BoxStream<'static, Result<Bytes, object_store::Error>>,
}

#[derive(Clone)]
pub struct Storage {
    store: Arc<dyn ObjectStore>,
    kind: &'static str,
}

impl Storage {
    /// The configured backend. `fs` creates its directory; nothing touches
    /// the network here.
    pub fn new(cfg: &StorageConfig) -> anyhow::Result<Self> {
        match cfg {
            StorageConfig::Fs { dir } => Self::local(dir),
            StorageConfig::S3(s3) => Self::s3(s3),
        }
    }

    /// A directory: atomic writes (temp file + rename) with fsync of the file
    /// and its directory.
    pub fn local(dir: &FsPath) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create INVOICE__STORAGE_DIR {}", dir.display()))?;
        let store = LocalFileSystem::new_with_prefix(dir)
            .with_context(|| format!("open INVOICE__STORAGE_DIR {}", dir.display()))?
            .with_fsync(true);
        Ok(Self {
            store: Arc::new(store),
            kind: "fs",
        })
    }

    pub fn s3(cfg: &S3Config) -> anyhow::Result<Self> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&cfg.bucket)
            .with_region(&cfg.region)
            .with_access_key_id(cfg.access_key_id.expose())
            .with_secret_access_key(cfg.secret_access_key.expose())
            .with_virtual_hosted_style_request(!cfg.path_style)
            .with_client_options(s3_client_options())
            .with_retry(RetryConfig {
                backoff: Default::default(),
                max_retries: S3_MAX_RETRIES,
                retry_timeout: S3_RETRY_TIMEOUT,
            });
        if let Some(endpoint) = cfg.request_endpoint() {
            builder = builder
                .with_allow_http(endpoint.starts_with("http://"))
                .with_endpoint(endpoint);
        }
        let store = builder.build().context("configure the S3 storage")?;
        Ok(Self {
            store: Arc::new(store),
            kind: "s3",
        })
    }

    /// `fs` or `s3`.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// The same backend with every key under `prefix/` (tests isolate
    /// themselves in one bucket this way).
    pub fn scoped(&self, prefix: &str) -> Result<Self, Error> {
        let store = PrefixStore::new(Arc::clone(&self.store), parse(prefix)?);
        Ok(Self {
            store: Arc::new(store),
            kind: self.kind,
        })
    }

    /// Store `bytes` under `key` (replacing it) and return their sha256.
    /// Readers see the old object or the new one, never a partial write.
    pub async fn put(&self, key: &str, bytes: Bytes) -> Result<String, Error> {
        let path = parse(key)?;
        let sha = sha256_hex(&bytes);
        self.store
            .put(&path, bytes.into())
            .await
            .map_err(|e| map_err(key, e))?;
        Ok(sha)
    }

    pub async fn get(&self, key: &str) -> Result<Bytes, Error> {
        let path = parse(key)?;
        let got = self.store.get(&path).await.map_err(|e| map_err(key, e))?;
        got.bytes().await.map_err(|e| map_err(key, e))
    }

    /// Streams the body; a missing object fails here, before the first byte.
    pub async fn get_stream(&self, key: &str) -> Result<Download, Error> {
        let path = parse(key)?;
        let got = self.store.get(&path).await.map_err(|e| map_err(key, e))?;
        Ok(Download {
            size: got.meta.size,
            stream: got.into_stream(),
        })
    }

    /// Server-side copy of `from` to `to` (replacing it): S3 `CopyObject`,
    /// on `fs` a hard link renamed into place — a later write of either key
    /// replaces its file and never changes the other.
    pub async fn copy(&self, from: &str, to: &str) -> Result<(), Error> {
        let (src, dst) = (parse(from)?, parse(to)?);
        self.store
            .copy(&src, &dst)
            .await
            .map_err(|e| map_err(from, e))
    }

    /// The sha256 of the object, streamed (never buffered whole).
    pub async fn sha256(&self, key: &str) -> Result<String, Error> {
        let mut stream = self.get_stream(key).await?.stream;
        let mut hasher = Sha256::new();
        while let Some(chunk) = stream.next().await {
            hasher.update(chunk.map_err(|e| map_err(key, e))?);
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }

    /// Idempotent: a missing object is not an error.
    pub async fn delete(&self, key: &str) -> Result<(), Error> {
        let path = parse(key)?;
        match self.store.delete(&path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(Error::Unavailable(e)),
        }
    }

    /// Best effort: the object of a transaction that did not commit, or one
    /// the database no longer points at.
    pub async fn remove(&self, key: &str) {
        if let Err(e) = self.delete(key).await {
            tracing::warn!(key, error = %e, "remove orphaned object");
        }
    }

    pub async fn exists(&self, key: &str) -> Result<bool, Error> {
        Ok(self.head(key).await?.is_some())
    }

    /// The object's listing entry, `None` when missing.
    pub async fn head(&self, key: &str) -> Result<Option<Object>, Error> {
        let path = parse(key)?;
        match self.store.head(&path).await {
            Ok(meta) => Ok(Some(Object::from_meta(meta))),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(Error::Unavailable(e)),
        }
    }

    /// Every object under the directory `prefix` (no trailing `/`),
    /// recursively, sorted by key.
    pub async fn list(&self, prefix: &str) -> Result<Vec<Object>, Error> {
        let path = parse(prefix)?;
        let mut out: Vec<Object> = self
            .store
            .list(Some(&path))
            .map_ok(Object::from_meta)
            .try_collect()
            .await
            .map_err(|e| map_err(prefix, e))?;
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    /// Delete every object under the directory `prefix` (a deleted space)
    /// with the backend's bulk delete (S3: batched `DeleteObjects`); returns
    /// how many were removed. Missing objects are not an error.
    pub async fn delete_prefix(&self, prefix: &str) -> Result<usize, Error> {
        let path = parse(prefix)?;
        let locations = self.store.list(Some(&path)).map_ok(|m| m.location).boxed();
        let mut deleted = self.store.delete_stream(locations);
        let mut n = 0;
        while let Some(r) = deleted.next().await {
            match r {
                Ok(_) => n += 1,
                Err(object_store::Error::NotFound { .. }) => {}
                Err(e) => return Err(Error::Unavailable(e)),
            }
        }
        Ok(n)
    }
}

fn s3_client_options() -> ClientOptions {
    ClientOptions::new()
        .with_timeout_disabled()
        .with_read_timeout(S3_READ_TIMEOUT)
        .with_connect_timeout(S3_CONNECT_TIMEOUT)
}

/// Keys never contain empty, `.` or `..` segments or control characters —
/// they cannot escape the storage root.
fn parse(key: &str) -> Result<Path, Error> {
    Path::parse(key)
        .ok()
        .filter(|p| p.parts().next().is_some())
        .ok_or_else(|| Error::InvalidKey(key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_abc() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn keys_are_validated() {
        assert!(parse("documents/2026/x.pdf").is_ok());
        assert!(parse("design/fonts/Brand Bold.ttf").is_ok());
        for bad in ["", "a//b", "../x", "a/./b", "a/../b", "a\nb"] {
            assert!(matches!(parse(bad), Err(Error::InvalidKey(_))), "{bad:?}");
        }
    }

    #[test]
    fn s3_bodies_have_no_total_timeout() {
        use object_store::ClientConfigKey as K;
        let opts = s3_client_options();
        assert_eq!(opts.get_config_value(&K::Timeout), None);
        assert!(opts.get_config_value(&K::ReadTimeout).is_some());
        assert!(opts.get_config_value(&K::ConnectTimeout).is_some());
    }

    #[test]
    fn errors_map_to_client_codes() {
        let unavailable = Error::Unavailable(object_store::Error::Generic {
            store: "S3",
            source: "connection refused".into(),
        });
        let (status, code) = AppError::from(unavailable).status_and_code();
        assert_eq!((status.as_u16(), code), (503, "storage_unavailable"));
        let (status, code) = AppError::from(Error::NotFound("k".into())).status_and_code();
        assert_eq!((status.as_u16(), code), (500, "internal"));
    }
}
