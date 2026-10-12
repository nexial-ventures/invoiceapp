//! The orphan sweep: document objects (`spaces/{id}/documents/…` of a space
//! that exists) that no `documents` row references (`pdf_path`,
//! `original_path`). Objects are written before the transaction that records
//! them, so a crash or a lost connection in between leaves one behind
//! (`docs/api/pdf.md`, Storage). Only objects older than [`MIN_AGE`] go: a
//! younger one may belong to an issue or upload still on its way to the
//! commit. Runs daily in `serve` (first [`FIRST_DELAY`] after the start) and
//! on demand as
//! `invoice storage sweep [--dry-run] [--force]`.
//!
//! Guard: a sweep that would delete with an empty reference set (a wrong
//! database or schema) or more than [`MAX_ORPHANS`] / 10 % of the examined
//! objects deletes nothing and logs an error, unless forced (CLI only).

use std::collections::HashSet;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use uuid::Uuid;

use super::Storage;

pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The first sweep after the start: not at once (a crash loop or a just
/// restored database gets an hour to be noticed), not a whole day (a
/// deployment restarted daily would never sweep).
pub const FIRST_DELAY: Duration = Duration::from_secs(60 * 60);
pub const MIN_AGE: Duration = Duration::from_secs(60 * 60);
/// Orphans one sweep may delete without `--force`: this many, or 10 % of
/// the examined objects when that is more.
pub const MAX_ORPHANS: usize = 100;
const SPACES: &str = "spaces";

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Objects last modified before it are old enough.
    pub cutoff: DateTime<Utc>,
    /// Only report.
    pub dry_run: bool,
    /// Delete even when the guard refuses.
    pub force: bool,
}

/// What one [`sweep`] found and did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Swept {
    /// Document objects of existing spaces examined.
    pub listed: usize,
    /// Unreferenced but younger than the cutoff (kept).
    pub young: usize,
    /// Unreferenced and old enough (deleted unless dry run or refused).
    pub orphans: Vec<String>,
    pub deleted: Vec<String>,
    /// Deletions that failed (logged; retried by the next sweep).
    pub failed: Vec<String>,
    /// Why the guard deleted nothing.
    pub refused: Option<String>,
}

/// The space id of `spaces/{id}/documents/…`.
fn document_space(key: &str) -> Option<Uuid> {
    let mut parts = key.split('/');
    if parts.next() != Some(SPACES) {
        return None;
    }
    let id = parts.next()?.parse().ok()?;
    (parts.next() == Some("documents") && parts.next().is_some()).then_some(id)
}

/// Why `orphans` of `listed` must not be deleted unforced, if so.
fn refusal(listed: usize, refs: usize, orphans: usize) -> Option<String> {
    if orphans == 0 {
        return None;
    }
    if refs == 0 {
        return Some(format!(
            "no document references any object, yet {listed} document object(s) exist \
             (wrong database or schema?)"
        ));
    }
    let max = MAX_ORPHANS.max(listed / 10);
    (orphans > max).then(|| format!("{orphans} orphan(s) of {listed} exceed the limit of {max}"))
}

async fn query(db: &DatabaseConnection, sql: &str) -> anyhow::Result<Vec<sea_orm::QueryResult>> {
    db.query_all(Statement::from_string(db.get_database_backend(), sql))
        .await
        .with_context(|| format!("sweep query: {sql}"))
}

/// The existing spaces and every storage key a `documents` row points at.
async fn references(db: &DatabaseConnection) -> anyhow::Result<(HashSet<Uuid>, HashSet<String>)> {
    let spaces = query(db, "SELECT id FROM spaces")
        .await?
        .iter()
        .map(|r| r.try_get_by_index::<Uuid>(0).context("spaces.id"))
        .collect::<anyhow::Result<_>>()?;
    let mut keys = HashSet::new();
    let sql = "SELECT space_id, pdf_path, original_path FROM documents \
               WHERE pdf_path IS NOT NULL OR original_path IS NOT NULL";
    for row in query(db, sql).await? {
        let space: Uuid = row.try_get_by_index(0).context("documents.space_id")?;
        for i in [1, 2] {
            let rel: Option<String> = row.try_get_by_index(i).context("documents path")?;
            if let Some(rel) = rel {
                keys.insert(format!("{SPACES}/{space}/{rel}"));
            }
        }
    }
    Ok((spaces, keys))
}

/// Sweep the root `storage`. The listing comes first, so a row committed
/// while it runs is still seen by the reference query.
pub async fn sweep(
    db: &DatabaseConnection,
    storage: &Storage,
    opts: Options,
) -> anyhow::Result<Swept> {
    let objects = storage
        .list(SPACES)
        .await
        .context("list the stored objects")?;
    let (spaces, refs) = references(db).await?;
    let mut swept = Swept::default();
    for o in objects {
        if !document_space(&o.key).is_some_and(|id| spaces.contains(&id)) {
            continue;
        }
        swept.listed += 1;
        if refs.contains(&o.key) {
            continue;
        }
        if o.version.last_modified >= opts.cutoff {
            swept.young += 1;
        } else {
            swept.orphans.push(o.key);
        }
    }
    swept.refused = refusal(swept.listed, refs.len(), swept.orphans.len());
    if let Some(why) = &swept.refused {
        tracing::error!(reason = %why, forced = opts.force, "orphan sweep guard");
        if !opts.force {
            return Ok(swept);
        }
    }
    if opts.dry_run {
        return Ok(swept);
    }
    for key in &swept.orphans {
        match storage.delete(key).await {
            Ok(()) => swept.deleted.push(key.clone()),
            Err(e) => {
                tracing::warn!(key = %key, error = %e, "orphan not deleted");
                swept.failed.push(key.clone());
            }
        }
    }
    Ok(swept)
}

/// The [`MIN_AGE`] cutoff, now.
pub fn cutoff_now() -> anyhow::Result<DateTime<Utc>> {
    Ok(Utc::now() - chrono::Duration::from_std(MIN_AGE).context("sweep cutoff")?)
}

/// Sweep every [`INTERVAL`], the first time [`FIRST_DELAY`] after the start
/// (a crash loop does not sweep); a failed round is logged and retried at
/// the next tick.
pub async fn run(db: DatabaseConnection, storage: Storage) {
    let start = tokio::time::Instant::now() + FIRST_DELAY;
    let mut tick = tokio::time::interval_at(start, INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let opts = match cutoff_now() {
            Ok(cutoff) => Options {
                cutoff,
                dry_run: false,
                force: false,
            },
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "orphan sweep failed");
                continue;
            }
        };
        match sweep(&db, &storage, opts).await {
            Ok(s) => tracing::info!(
                listed = s.listed,
                young = s.young,
                orphans = s.orphans.len(),
                deleted = s.deleted.len(),
                failed = s.failed.len(),
                refused = s.refused.is_some(),
                "orphaned document objects swept"
            ),
            Err(e) => tracing::warn!(error = %format!("{e:#}"), "orphan sweep failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "3f2a0c1e-0000-4000-8000-000000000001";

    #[test]
    fn only_document_keys_of_a_space() {
        for key in [
            format!("spaces/{ID}/documents/2026/x.pdf"),
            format!("spaces/{ID}/documents/x.pdf"),
        ] {
            assert_eq!(document_space(&key).map(|u| u.to_string()), Some(ID.into()));
        }
        for key in [
            format!("spaces/{ID}/design/logo.svg"),
            format!("spaces/{ID}/email/templates/cs.json"),
            format!("spaces/{ID}/documents"),
            "spaces/not-a-uuid/documents/x.pdf".into(),
            "documents/2026/x.pdf".into(),
            format!("other/{ID}/documents/x.pdf"),
        ] {
            assert_eq!(document_space(&key), None, "{key}");
        }
    }

    #[test]
    fn the_guard() {
        assert_eq!(refusal(5, 0, 0), None, "nothing to delete");
        assert!(refusal(5, 0, 1).is_some_and(|r| r.contains("no document references")));
        assert_eq!(refusal(150, 10, 100), None);
        assert!(refusal(150, 10, 101).is_some_and(|r| r.contains("limit of 100")));
        assert_eq!(refusal(5000, 10, 500), None, "10 % of a large store");
        assert!(refusal(5000, 10, 501).is_some());
    }
}
