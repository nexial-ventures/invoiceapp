//! The orphan sweep: document objects (`spaces/*/documents/…`) that no
//! `documents` row references (`pdf_path`, `original_path`). Objects are
//! written before the transaction that records them, so a crash or a lost
//! connection in between leaves one behind (`docs/api/pdf.md`, Storage).
//! Only objects older than [`MIN_AGE`] go: a younger one may belong to an
//! issue or upload still on its way to the commit. Runs daily in `serve`
//! and on demand as `invoice storage sweep [--dry-run]`.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use uuid::Uuid;

use super::Storage;

pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub const MIN_AGE: Duration = Duration::from_secs(60 * 60);
const SPACES: &str = "spaces";

/// What one [`sweep`] found and did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Swept {
    /// Document objects examined.
    pub listed: usize,
    /// Unreferenced but younger than the cutoff (kept).
    pub young: usize,
    /// Unreferenced and old enough: deleted, or only reported on a dry run.
    pub orphans: Vec<String>,
    pub deleted: usize,
    /// Deletions that failed (logged; retried by the next sweep).
    pub failed: usize,
}

/// `spaces/{id}/documents/…`.
fn is_document_key(key: &str) -> bool {
    let mut parts = key.split('/');
    parts.next() == Some(SPACES)
        && parts.next().is_some_and(|id| !id.is_empty())
        && parts.next() == Some("documents")
        && parts.next().is_some()
}

/// Every storage key a `documents` row points at.
async fn referenced(db: &DatabaseConnection) -> anyhow::Result<HashSet<String>> {
    let rows = db
        .query_all(Statement::from_string(
            db.get_database_backend(),
            "SELECT space_id, pdf_path, original_path FROM documents \
             WHERE pdf_path IS NOT NULL OR original_path IS NOT NULL",
        ))
        .await
        .context("read the referenced document objects")?;
    let mut keys = HashSet::new();
    for row in rows {
        let space: Uuid = row.try_get_by_index(0).context("documents.space_id")?;
        for i in [1, 2] {
            let rel: Option<String> = row.try_get_by_index(i).context("documents path")?;
            if let Some(rel) = rel {
                keys.insert(format!("{SPACES}/{space}/{rel}"));
            }
        }
    }
    Ok(keys)
}

/// Delete (unless `dry_run`) every unreferenced document object of the root
/// `storage` last modified before `cutoff`. The listing comes first, so a
/// row committed while it runs is still seen by the reference query.
pub async fn sweep(
    db: &DatabaseConnection,
    storage: &Storage,
    cutoff: DateTime<Utc>,
    dry_run: bool,
) -> anyhow::Result<Swept> {
    let objects = storage
        .list(SPACES)
        .await
        .context("list the stored objects")?;
    let refs = referenced(db).await?;
    let mut swept = Swept::default();
    for o in objects.into_iter().filter(|o| is_document_key(&o.key)) {
        swept.listed += 1;
        if refs.contains(&o.key) {
            continue;
        }
        if o.version.last_modified >= cutoff {
            swept.young += 1;
            continue;
        }
        if !dry_run {
            match storage.delete(&o.key).await {
                Ok(()) => swept.deleted += 1,
                Err(e) => {
                    tracing::warn!(key = %o.key, error = %e, "orphan not deleted");
                    swept.failed += 1;
                }
            }
        }
        swept.orphans.push(o.key);
    }
    Ok(swept)
}

/// [`sweep`] with the [`MIN_AGE`] cutoff, now.
pub async fn sweep_now(
    db: &DatabaseConnection,
    storage: &Storage,
    dry_run: bool,
) -> anyhow::Result<Swept> {
    let cutoff = Utc::now() - chrono::Duration::from_std(MIN_AGE).context("sweep cutoff")?;
    sweep(db, storage, cutoff, dry_run).await
}

/// Sweep every [`INTERVAL`], the first time right away; a failed round is
/// logged and retried at the next tick.
pub async fn run(db: DatabaseConnection, storage: Storage) {
    let mut tick = tokio::time::interval(INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match sweep_now(&db, &storage, false).await {
            Ok(s) => tracing::info!(
                listed = s.listed,
                young = s.young,
                deleted = s.deleted,
                failed = s.failed,
                "orphaned document objects swept"
            ),
            Err(e) => tracing::warn!(error = %format!("{e:#}"), "orphan sweep failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_document_keys_of_a_space() {
        for key in [
            "spaces/3f2a/documents/2026/x.pdf",
            "spaces/3f2a/documents/x.pdf",
        ] {
            assert!(is_document_key(key), "{key}");
        }
        for key in [
            "spaces/3f2a/design/logo.svg",
            "spaces/3f2a/email/templates/cs.json",
            "spaces/3f2a/documents",
            "spaces//documents/x.pdf",
            "documents/2026/x.pdf",
            "other/3f2a/documents/x.pdf",
        ] {
            assert!(!is_document_key(key), "{key}");
        }
    }
}
