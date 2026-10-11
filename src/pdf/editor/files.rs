//! Storage side of the editor: the listings of live / draft / versions,
//! applying a draft, replacing the live set with a version, snapshots and
//! cleanup. Copies are server-side; keys are relative to the space prefix.

use std::collections::BTreeMap;

use futures_util::future::try_join3;
use uuid::Uuid;

use super::{DRAFT_PREFIX, VERSIONS_PREFIX, VersionFile, draft_key, version_prefix};
use crate::pdf::design::{self, key as live_key};
use crate::storage::{DESIGN_PREFIX, Error, Object, Storage, Version};

/// One listing of each editor directory (fetched concurrently).
pub struct Stored {
    pub live: Vec<Object>,
    pub draft: Vec<Object>,
    pub versions: Vec<Object>,
}

impl Stored {
    pub async fn load(storage: &Storage) -> Result<Self, Error> {
        let (live, draft, versions) = try_join3(
            storage.list(DESIGN_PREFIX),
            storage.list(DRAFT_PREFIX),
            storage.list(VERSIONS_PREFIX),
        )
        .await?;
        Ok(Self {
            live,
            draft,
            versions,
        })
    }

    /// Bytes stored for live + draft + versions (hidden objects included).
    pub fn usage(&self) -> u64 {
        [&self.live, &self.draft, &self.versions]
            .into_iter()
            .flatten()
            .map(|o| o.version.size)
            .sum()
    }

    pub fn live_files(&self) -> BTreeMap<String, Object> {
        design::by_path(self.live.clone(), DESIGN_PREFIX)
    }

    pub fn draft_files(&self) -> BTreeMap<String, Object> {
        design::by_path(self.draft.clone(), DRAFT_PREFIX)
    }
}

/// Make the draft live: remove the deleted paths, then copy every draft
/// file over its live path (a draft file wins over a stale deletion).
pub async fn apply_draft(
    storage: &Storage,
    draft: &BTreeMap<String, Object>,
    deleted: &[String],
) -> Result<(), Error> {
    for path in deleted {
        storage.delete(&live_key(path)).await?;
    }
    for path in draft.keys() {
        storage.copy(&draft_key(path), &live_key(path)).await?;
    }
    Ok(())
}

/// The version's recorded files are all stored with their recorded size.
pub async fn complete(storage: &Storage, id: Uuid, files: &[VersionFile]) -> Result<bool, Error> {
    let stored = design::stored(storage, &version_prefix(id)).await?;
    Ok(files.iter().all(|f| {
        stored
            .get(&f.path)
            .is_some_and(|o| o.version.size == f.size)
    }))
}

/// Make the version's files the live set: copy them over and delete every
/// other live file.
pub async fn replace_live(storage: &Storage, id: Uuid, files: &[VersionFile]) -> Result<(), Error> {
    let prefix = version_prefix(id);
    for path in design::stored(storage, DESIGN_PREFIX).await?.keys() {
        if !files.iter().any(|f| &f.path == path) {
            storage.delete(&live_key(path)).await?;
        }
    }
    for f in files {
        storage
            .copy(&format!("{prefix}/{}", f.path), &live_key(&f.path))
            .await?;
    }
    Ok(())
}

/// Copy the live set to `design-versions/{id}/` (sha256 streamed from the
/// copy); a failure removes the partial copy again (best effort).
pub async fn snapshot(storage: &Storage, id: Uuid) -> Result<Vec<VersionFile>, Error> {
    let prefix = version_prefix(id);
    let result = copy_live(storage, &prefix).await;
    if result.is_err() {
        remove_prefix(storage, &prefix).await;
    }
    result
}

async fn copy_live(storage: &Storage, prefix: &str) -> Result<Vec<VersionFile>, Error> {
    let mut files = Vec::new();
    for (path, o) in design::stored(storage, DESIGN_PREFIX).await? {
        let key = format!("{prefix}/{path}");
        storage.copy(&o.key, &key).await?;
        let sha256 = storage.sha256(&key).await?;
        files.push(VersionFile {
            path,
            size: o.version.size,
            sha256,
        });
    }
    Ok(files)
}

/// Delete version objects whose directory belongs to no row (`referenced`):
/// leftovers of a run whose commit failed. Only under the lock — an
/// in-flight snapshot has no row yet. Best effort.
pub async fn collect_orphans(storage: &Storage, referenced: &[Uuid]) {
    let objects = match storage.list(VERSIONS_PREFIX).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "list design versions for cleanup");
            return;
        }
    };
    for o in objects {
        if !referenced.iter().any(|id| owned_by(&o.key, *id)) {
            storage.remove(&o.key).await;
        }
    }
}

/// After the commit: the objects of pruned versions (best effort).
pub async fn remove_versions(storage: &Storage, ids: &[Uuid]) {
    for id in ids {
        remove_prefix(storage, &version_prefix(*id)).await;
    }
}

/// After the commit: the applied draft files, unless one was replaced
/// meanwhile by a new upload (its listed version differs). Best effort.
pub async fn remove_applied(storage: &Storage, draft: &BTreeMap<String, Object>) {
    for o in draft.values() {
        match storage.head(&o.key).await {
            Ok(Some(now)) if same(&now.version, &o.version) => storage.remove(&o.key).await,
            Ok(_) => {}
            Err(e) => tracing::warn!(key = o.key, error = %e, "remove applied draft file"),
        }
    }
}

/// A `HEAD` and a listing of the same object: S3 reports `Last-Modified`
/// in whole seconds on one and with milliseconds on the other, so the ETag
/// decides when there is one.
fn same(a: &Version, b: &Version) -> bool {
    match (&a.e_tag, &b.e_tag) {
        (Some(x), Some(y)) => x == y && a.size == b.size,
        _ => a == b,
    }
}

fn owned_by(key: &str, id: Uuid) -> bool {
    key.starts_with(&format!("{}/", version_prefix(id)))
}

pub async fn remove_prefix(storage: &Storage, prefix: &str) {
    if let Err(e) = storage.delete_prefix(prefix).await {
        tracing::warn!(prefix, error = %e, "remove design objects");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_object_versions() {
        let t = chrono::Utc::now();
        let v = |e: Option<&str>, size, ms| Version {
            e_tag: e.map(str::to_string),
            size,
            last_modified: t + chrono::Duration::milliseconds(ms),
        };
        assert!(same(&v(Some("a"), 1, 0), &v(Some("a"), 1, 400)));
        assert!(!same(&v(Some("a"), 1, 0), &v(Some("b"), 1, 0)));
        assert!(same(&v(None, 1, 0), &v(None, 1, 0)));
        assert!(!same(&v(None, 1, 0), &v(None, 1, 400)));
    }

    #[test]
    fn version_ownership() {
        let id = Uuid::nil();
        let key = format!("{}/logo.svg", version_prefix(id));
        assert!(owned_by(&key, id));
        assert!(!owned_by(&key, Uuid::max()));
        assert!(!owned_by(
            "design-versions/00000000-0000-0000-0000-0000000000001/x",
            id
        ));
    }
}
