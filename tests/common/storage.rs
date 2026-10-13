//! Storage backends for tests: a throwaway directory (`fs`) or a random key
//! prefix in the S3 test bucket (`s3`, from `TEST_S3_*`), deleted on drop.

use std::path::Path;

use invoiceapp::secret::Secret;
use invoiceapp::storage::{S3Config, Storage};

const S3_VARS: [&str; 5] = [
    "TEST_S3_ENDPOINT",
    "TEST_S3_BUCKET",
    "TEST_S3_REGION",
    "TEST_S3_ACCESS_KEY_ID",
    "TEST_S3_SECRET_ACCESS_KEY",
];

fn s3_var(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| {
            panic!(
                "{name} must be set for the S3 storage tests \
                 (all of {S3_VARS:?}; see .env.example)"
            )
        })
}

/// The S3 test bucket, unscoped.
pub fn s3_bucket() -> Storage {
    let cfg = S3Config {
        endpoint: Some(s3_var("TEST_S3_ENDPOINT")),
        bucket: s3_var("TEST_S3_BUCKET"),
        region: s3_var("TEST_S3_REGION"),
        access_key_id: Secret::new(s3_var("TEST_S3_ACCESS_KEY_ID")),
        secret_access_key: Secret::new(s3_var("TEST_S3_SECRET_ACCESS_KEY")),
        path_style: true,
    };
    Storage::s3(&cfg).expect("build the S3 test storage")
}

/// An S3 storage whose endpoint refuses connections.
pub fn dead_s3() -> Storage {
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("free port");
    let cfg = S3Config {
        endpoint: Some(format!("http://{addr}")),
        bucket: "invoice-test".into(),
        region: "garage".into(),
        access_key_id: Secret::new("GKdead".into()),
        secret_access_key: Secret::new("dead".into()),
        path_style: true,
    };
    Storage::s3(&cfg).expect("build the dead S3 storage")
}

pub struct TestStorage {
    pub storage: Storage,
    dir: Option<tempfile::TempDir>,
    /// The S3 prefix to delete on drop.
    s3: Option<String>,
}

impl TestStorage {
    pub fn fs() -> Self {
        let dir = tempfile::tempdir().expect("storage dir");
        let storage = Storage::local(dir.path()).expect("fs storage");
        Self {
            storage,
            dir: Some(dir),
            s3: None,
        }
    }

    pub fn s3() -> Self {
        let bucket = s3_bucket();
        let prefix = format!("test-{}", uuid::Uuid::new_v4());
        let storage = bucket.scoped(&prefix).expect("scoped S3 storage");
        Self {
            storage,
            dir: None,
            s3: Some(prefix),
        }
    }

    pub fn kind(&self) -> &'static str {
        self.storage.kind()
    }

    /// The fs root (fs tests inspect files directly).
    pub fn path(&self) -> &Path {
        self.dir.as_ref().expect("an fs test storage").path()
    }

    /// The stored object, `None` when missing.
    pub async fn bytes(&self, key: &str) -> Option<Vec<u8>> {
        match self.storage.get(key).await {
            Ok(b) => Some(b.to_vec()),
            Err(invoiceapp::storage::Error::NotFound(_)) => None,
            Err(e) => panic!("get {key}: {e}"),
        }
    }

    pub async fn keys(&self, prefix: &str) -> Vec<String> {
        self.storage
            .list(prefix)
            .await
            .expect("list")
            .into_iter()
            .map(|o| o.key)
            .collect()
    }
}

impl Drop for TestStorage {
    fn drop(&mut self) {
        let Some(prefix) = self.s3.take() else {
            return;
        };
        // Dedicated thread + runtime: works under any test runtime flavor.
        // A fresh client too — the test's pooled connections belong to its
        // (now blocked) runtime.
        let _ = std::thread::spawn(move || {
            let bucket = s3_bucket();
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                if let Ok(objects) = bucket.list(&prefix).await {
                    for o in objects {
                        bucket.remove(&o.key).await;
                    }
                }
            });
        })
        .join();
    }
}
