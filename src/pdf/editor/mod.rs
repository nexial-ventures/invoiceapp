//! Editing a space's PDF design from the UI (phase 4f,
//! `docs/api/design.md`): a draft (`design-draft/…` + the deletions in
//! `design_drafts`) previewed and validated before it becomes live, and a
//! history of activations (`design-versions/{id}/…` + `design_versions`).
//!
//! Every change takes the space's design lock (the `design_drafts` row
//! `FOR UPDATE NOWAIT`; held → 409 `design_busy`). The slow renders of an
//! activation / restore check run before it, and the locked part only
//! applies a draft whose revision is still the one checked.

pub mod activate;
pub mod entity;
mod files;
pub mod handlers;
pub mod preview;
mod repo;
mod usage;

use axum::Router;
use axum::routing::{delete, get, post, put};
use chrono::{DateTime, FixedOffset};
use sea_orm::{DatabaseConnection, DatabaseTransaction, TransactionTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::design::QR_IMAGE;
use crate::app::AppState;
use crate::error::AppError;
use crate::space::SpaceId;

pub use handlers::DesignListing;
pub use usage::{ensure_usage, refresh_all, refresh_usage};

/// The draft files, relative to the space prefix.
pub const DRAFT_PREFIX: &str = "design-draft";
/// One directory per kept version: `design-versions/{id}/…`.
pub const VERSIONS_PREFIX: &str = "design-versions";
/// Live + draft + kept versions, by stored object size.
pub const QUOTA_BYTES: u64 = 50 * 1024 * 1024;
/// Versions kept per space; older rows and objects are deleted.
pub const KEEP_VERSIONS: u64 = 10;

const EXTENSIONS: [&str; 9] = [
    "typ", "toml", "ttf", "otf", "woff2", "svg", "png", "jpg", "jpeg",
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/design", get(handlers::design))
        .route("/design/file", get(handlers::file))
        .route(
            "/design/draft/file",
            put(handlers::put_file).delete(handlers::delete_file),
        )
        .route("/design/draft", delete(handlers::discard))
        .route("/design/draft/preview", get(preview::preview))
        .route("/design/activate", post(activate::activate))
        .route("/design/versions", get(activate::versions))
        .route("/design/versions/{id}/restore", post(activate::restore))
        .route("/design/reset", post(activate::reset))
}

/// An upload target. A new path: 1–2 lowercase `[a-z0-9._-]` segments, no
/// hidden segment (the loader would skip it), an allowed extension. A path
/// of an existing file (`existing`: default, live and draft paths) may be
/// replaced whatever its name (`fonts/Inter-Regular.ttf`, CLI-pushed
/// files). Never `qr.svg`, never a file/directory clash with an existing
/// path (`fonts` vs `fonts/a.ttf`).
pub fn valid_upload_path<'a>(path: &str, existing: impl Iterator<Item = &'a str> + Clone) -> bool {
    let known = existing.clone().any(|p| p == path);
    let clash = existing.into_iter().any(|p| {
        p.strip_prefix(path).is_some_and(|r| r.starts_with('/'))
            || path.strip_prefix(p).is_some_and(|r| r.starts_with('/'))
    });
    path != QR_IMAGE && !clash && (strict_path(path) || (known && safe_path(path)))
}

fn strict_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    let ext_ok = path
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.ends_with('/') && EXTENSIONS.contains(&ext));
    (1..=2).contains(&segments.len())
        && segments.iter().all(|s| {
            !s.is_empty()
                && !s.starts_with('.')
                && s.bytes()
                    .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
        })
        && ext_ok
}

/// A path that can name an existing file (download, draft deletion): any
/// safe relative key — live files pushed by the CLI may use other names
/// (`fonts/Inter-Regular.ttf` of the default).
pub fn safe_path(path: &str) -> bool {
    path.split('/').all(|s| {
        !s.is_empty() && !s.starts_with('.') && !s.chars().any(|c| c.is_control() || c == '\\')
    })
}

pub fn draft_key(path: &str) -> String {
    format!("{DRAFT_PREFIX}/{path}")
}

pub fn version_prefix(id: Uuid) -> String {
    format!("{VERSIONS_PREFIX}/{id}")
}

/// Who made a change; `null` once the account is deleted.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserRef {
    pub email: String,
    pub display_name: String,
}

/// A file of a version (`design_versions.files`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VersionFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: Uuid,
    /// 1.. per space.
    pub number: i32,
    /// `activation` | `revert` | `default`.
    pub kind: String,
    pub created_at: DateTime<FixedOffset>,
    pub created_by: Option<UserRef>,
    pub files: Vec<VersionFile>,
    /// The newest version (the live state).
    pub current: bool,
}

/// Take the space's design lock without waiting: a transaction holding the
/// draft row (created on first use), or 409 `design_busy`.
async fn locked(
    db: &DatabaseConnection,
    space: SpaceId,
) -> Result<(DatabaseTransaction, entity::draft::Model), AppError> {
    repo::ensure(db, space).await?;
    let txn = begin(db).await?;
    match repo::lock(&txn, space).await {
        Ok(row) => Ok((txn, row)),
        Err(e) => {
            rollback(txn).await;
            Err(e)
        }
    }
}

/// Await the commit (or the rollback on an error — a dropped transaction
/// would only queue it and keep the design lock).
async fn in_txn<T>(txn: DatabaseTransaction, result: Result<T, AppError>) -> Result<T, AppError> {
    match result {
        Ok(v) => {
            txn.commit().await?;
            Ok(v)
        }
        Err(e) => {
            rollback(txn).await;
            Err(e)
        }
    }
}

async fn rollback(txn: DatabaseTransaction) {
    if let Err(e) = txn.rollback().await {
        tracing::warn!(error = %e, "design transaction rollback failed");
    }
}

async fn begin(db: &DatabaseConnection) -> Result<DatabaseTransaction, AppError> {
    Ok(db.begin().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(path: &str) -> bool {
        valid_upload_path(
            path,
            ["invoice.typ", "fonts/Inter-Regular.ttf", "fonts/OFL.txt"].into_iter(),
        )
    }

    #[test]
    fn upload_paths() {
        for ok in [
            "invoice.typ",
            "logo.svg",
            "fonts/inter.ttf",
            "fonts/brand-bold_2.woff2",
            "img/qr.svg",
            "theme.toml",
            "photo.jpeg",
        ] {
            assert!(upload(ok), "{ok}");
        }
        for bad in [
            "",
            "qr.svg",
            "Logo.svg",
            "a/b/c.png",
            "../x.png",
            "fonts/../x.png",
            ".hidden.png",
            "fonts/.x.ttf",
            "logo",
            "logo.gif",
            "fonts/other.txt",
            "a//b.png",
            "/logo.png",
            "logo.png/",
            "fonts/.ttf",
            "lo go.png",
            "lógo.png",
        ] {
            assert!(!upload(bad), "{bad}");
        }
        // Existing files keep their names; clashes and `qr.svg` never pass.
        assert!(upload("fonts/Inter-Regular.ttf"));
        assert!(upload("fonts/OFL.txt"));
        assert!(!valid_upload_path("qr.svg", ["qr.svg"].into_iter()));
        assert!(!valid_upload_path(
            "logo.svg/x.png",
            ["logo.svg"].into_iter()
        ));
        assert!(!valid_upload_path("img.png", ["img.png/a.png"].into_iter()));
        assert!(!valid_upload_path("fonts", ["fonts/a.ttf"].into_iter()));
        assert!(!valid_upload_path(
            "Fonts/X.TTF",
            ["fonts/x.ttf"].into_iter()
        ));
    }

    #[test]
    fn safe_paths() {
        assert!(safe_path("fonts/Inter-Regular.ttf"));
        assert!(safe_path("a/b/c d.png"));
        for bad in ["", "../x", "a/../b", "a//b", ".git/config", "a\nb", "a\\b"] {
            assert!(!safe_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn keys() {
        assert_eq!(draft_key("fonts/a.ttf"), "design-draft/fonts/a.ttf");
        let id = Uuid::nil();
        assert_eq!(
            version_prefix(id),
            "design-versions/00000000-0000-0000-0000-000000000000"
        );
    }
}
