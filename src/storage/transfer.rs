//! The CLI's file transfers: `invoiceapp storage migrate` and `invoiceapp design
//! push` copy local files into the configured backend (verified by sha256,
//! the source only read); `invoiceapp design pull` copies a prefix out.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use md5::{Digest as _, Md5};

use super::local::{self, Walk};
use super::{DESIGN_PREFIX, Storage, Version, sha256_hex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Written (new, or replacing different content when overwriting).
    Copied,
    /// Already stored with the same sha256.
    Identical,
    /// Stored with different content and left alone.
    Differs,
}

/// Whether `local` matches the stored object, decided without downloading
/// it: a different size → no; an MD5 ETag (a single-part S3 upload) →
/// compare it; otherwise `None` (the fs backend's ETag is no digest).
pub fn same_without_download(local: &[u8], stored: &Version) -> Option<bool> {
    if stored.size != local.len() as u64 {
        return Some(false);
    }
    let etag = stored.e_tag.as_deref()?.trim_matches('"');
    let is_md5 = etag.len() == 32 && etag.bytes().all(|b| b.is_ascii_hexdigit());
    is_md5.then(|| {
        let md5: String = Md5::digest(local)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        etag.eq_ignore_ascii_case(&md5)
    })
}

/// Copy `path` to `key`. Identical content is skipped, so a re-run is a
/// no-op (and downloads nothing when the backend's ETag decides it);
/// different content is replaced only with `overwrite`. Every write is read
/// back and compared by sha256.
pub async fn copy_file(
    storage: &Storage,
    key: &str,
    path: &Path,
    overwrite: bool,
) -> Result<Outcome> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let sha = sha256_hex(&bytes);
    if let Some(existing) = storage.head(key).await? {
        let same = match same_without_download(&bytes, &existing.version) {
            Some(same) => same,
            None => sha256_hex(&storage.get(key).await?) == sha,
        };
        if same {
            return Ok(Outcome::Identical);
        }
        if !overwrite {
            return Ok(Outcome::Differs);
        }
    }
    storage.put(key, bytes.into()).await?;
    let stored = sha256_hex(&storage.get(key).await?);
    if stored != sha {
        bail!("{key}: stored sha256 {stored} differs from the source {sha}");
    }
    Ok(Outcome::Copied)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub copied: Vec<String>,
    pub identical: Vec<String>,
    /// Keys stored with other content (never overwritten), failed copies and
    /// local entries that could not be read.
    pub problems: Vec<String>,
}

impl Report {
    pub fn summary(&self) -> String {
        format!(
            "{} copied, {} already identical, {} problem(s)",
            self.copied.len(),
            self.identical.len(),
            self.problems.len()
        )
    }
}

async fn walk(dir: &Path) -> Result<Walk> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || local::walk(&dir))
        .await
        .context("directory walk panicked")?
}

/// Copy every (non-hidden) file of `dir` under `prefix`.
pub async fn copy_tree(
    storage: &Storage,
    dir: &Path,
    prefix: Option<&str>,
    overwrite: bool,
    report: &mut Report,
) -> Result<()> {
    let walked = walk(dir).await?;
    report
        .problems
        .extend(walked.skipped.into_iter().map(|s| format!("skipped {s}")));
    let files: Vec<(String, PathBuf)> = walked
        .files
        .into_iter()
        .map(|f| match prefix {
            Some(p) => (format!("{p}/{}", f.rel), f.path),
            None => (f.rel, f.path),
        })
        .collect();
    for (key, path) in files {
        match copy_file(storage, &key, &path, overwrite).await {
            Ok(Outcome::Copied) => report.copied.push(key),
            Ok(Outcome::Identical) => report.identical.push(key),
            Ok(Outcome::Differs) => report.problems.push(format!(
                "{key}: already stored with different content, kept"
            )),
            Err(e) => report.problems.push(format!("{key}: {e:#}")),
        }
    }
    Ok(())
}

/// `--from-dir` (the old `INVOICEAPP__STORAGE_DIR`, keys = relative paths) and
/// optionally `--design-dir` (the old `INVOICEAPP__DESIGN_DIR`, under `design/`).
/// Idempotent; existing different objects are reported, never replaced.
pub async fn migrate(
    storage: &Storage,
    from_dir: &Path,
    design_dir: Option<&Path>,
) -> Result<Report> {
    let mut report = Report::default();
    copy_tree(storage, from_dir, None, false, &mut report).await?;
    if let Some(dir) = design_dir {
        copy_tree(storage, dir, Some(DESIGN_PREFIX), false, &mut report).await?;
    }
    Ok(report)
}

/// Write every object under `prefix` into `dir` (created; existing files
/// replaced). Returns the written relative paths.
pub async fn pull(storage: &Storage, prefix: &str, dir: &Path) -> Result<Vec<String>> {
    let mut written = Vec::new();
    for o in storage.list(prefix).await? {
        let Some(rel) = local::rel_key(&o.key, prefix) else {
            continue;
        };
        let dest = rel.split('/').fold(dir.to_path_buf(), |p, s| p.join(s));
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let bytes = storage.get(&o.key).await?;
        tokio::fs::write(&dest, &bytes)
            .await
            .with_context(|| format!("write {}", dest.display()))?;
        written.push(rel.to_string());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(size: u64, e_tag: Option<&str>) -> Version {
        Version {
            e_tag: e_tag.map(str::to_string),
            size,
            last_modified: chrono::DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn compares_without_downloading_when_the_etag_is_an_md5() {
        // md5("abc")
        let md5 = "\"900150983cd24fb0d6963f7d28e17f72\"";
        assert_eq!(
            same_without_download(b"abc", &version(3, Some(md5))),
            Some(true)
        );
        assert_eq!(
            same_without_download(b"abd", &version(3, Some(md5))),
            Some(false)
        );
        assert_eq!(
            same_without_download(b"abcd", &version(3, None)),
            Some(false)
        );
        // Multipart and fs ETags are no MD5 of the content: download needed.
        let multipart = "\"900150983cd24fb0d6963f7d28e17f72-2\"";
        assert_eq!(
            same_without_download(b"abc", &version(3, Some(multipart))),
            None
        );
        assert_eq!(
            same_without_download(b"abc", &version(3, Some("2a1f-18c-3"))),
            None
        );
        assert_eq!(same_without_download(b"abc", &version(3, None)), None);
    }
}
