//! The design file set: the default design embedded from `design/`,
//! overridden file by file by the space's storage keys `design/…` (the
//! listing reused for a few seconds and file contents cached by key +
//! version in the bounded [`Cache`], so unchanged files are not downloaded
//! again). The embedded files' digests are computed once per process.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use anyhow::Context as _;
use bytes::Bytes;
use mdcast_client::mdcast_api::Digest;
use rust_embed::Embed;
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::AppError;
use crate::storage::local::{is_hidden, rel_key};
use crate::storage::{DESIGN_PREFIX, Object, Storage};

pub use super::design_cache::Cache;
use super::design_cache::scoped_key;

#[derive(Embed)]
#[folder = "design/"]
struct DefaultDesign;

pub const TEMPLATE: &str = "invoice.typ";
/// Generated per render; a design file of this name is ignored.
pub const QR_IMAGE: &str = "qr.svg";
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Custom,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DesignFile {
    pub path: String,
    pub source: Source,
    pub size: u64,
}

/// The design path of a storage key: `design/fonts/a.ttf` → `fonts/a.ttf`;
/// `None` outside the prefix or for hidden segments.
pub fn design_path(key: &str) -> Option<&str> {
    rel_key(key, DESIGN_PREFIX)
}

pub fn key(path: &str) -> String {
    format!("{DESIGN_PREFIX}/{path}")
}

/// Remove the custom file `path` (`invoice design rm`); `false` when none is
/// stored (a built-in default cannot be removed).
pub async fn remove(storage: &Storage, path: &str) -> Result<bool, crate::storage::Error> {
    let key = key(path);
    if storage.head(&key).await?.is_none() {
        return Ok(false);
    }
    storage.delete(&key).await?;
    Ok(true)
}

/// An embedded default file and its mdcast digest.
pub(crate) struct Embedded {
    pub bytes: Bytes,
    pub digest: Digest,
}

/// Hashed once: the default fonts alone are ~1.7 MB, and every render
/// names each file by digest. Debug builds read the embedded folder from
/// disk, so an edit of `design/` needs a restart there.
static EMBEDDED: LazyLock<BTreeMap<String, Embedded>> = LazyLock::new(|| {
    DefaultDesign::iter()
        .filter(|p| !p.split('/').any(is_hidden))
        .filter_map(|p| {
            let bytes = match DefaultDesign::get(&p)?.data {
                Cow::Borrowed(b) => Bytes::from_static(b),
                Cow::Owned(v) => Bytes::from(v),
            };
            let digest = mdcast_client::sha256(&bytes);
            Some((p.into_owned(), Embedded { bytes, digest }))
        })
        .collect()
});

pub(crate) fn embedded_files() -> &'static BTreeMap<String, Embedded> {
    &EMBEDDED
}

/// The embedded default files by path, with their sizes.
pub(crate) fn embedded() -> BTreeMap<String, u64> {
    EMBEDDED
        .iter()
        .map(|(p, e)| (p.clone(), e.bytes.len() as u64))
        .collect()
}

pub(crate) fn embedded_bytes(path: &str) -> anyhow::Result<Bytes> {
    Ok(EMBEDDED
        .get(path)
        .with_context(|| format!("embedded design file {path} vanished"))?
        .bytes
        .clone())
}

/// The stored files under the directory `prefix` by relative path (hidden
/// ones skipped).
pub(crate) async fn stored(
    storage: &Storage,
    prefix: &str,
) -> Result<BTreeMap<String, Object>, crate::storage::Error> {
    Ok(by_path(storage.list(prefix).await?, prefix))
}

/// A listing of `prefix` by relative path (hidden ones skipped).
pub(crate) fn by_path(objects: Vec<Object>, prefix: &str) -> BTreeMap<String, Object> {
    objects
        .into_iter()
        .filter_map(|o| Some((rel_key(&o.key, prefix)?.to_string(), o)))
        .collect()
}

/// Which stored files override the embedded default: the directory `base`
/// minus the paths in `deleted`, then `top` over it. Renders use
/// [`Layers::LIVE`]; the draft preview / activation check put the draft on
/// top of the live set, a restore check uses a version as `base`.
#[derive(Debug, Clone, Copy)]
pub struct Layers<'a> {
    pub base: &'a str,
    pub deleted: &'a [String],
    pub top: Option<&'a str>,
}

impl Layers<'static> {
    pub const LIVE: Layers<'static> = Layers {
        base: DESIGN_PREFIX,
        deleted: &[],
        top: None,
    };
}

impl Layers<'_> {
    fn is_live(&self) -> bool {
        self.base == DESIGN_PREFIX && self.top.is_none() && self.deleted.is_empty()
    }

    async fn overrides(&self, storage: &Storage) -> Result<BTreeMap<String, Object>, AppError> {
        let mut files = stored(storage, self.base).await?;
        files.retain(|path, _| !self.deleted.contains(path));
        if let Some(top) = self.top {
            files.extend(stored(storage, top).await?);
        }
        Ok(files)
    }
}

/// The effective set for `GET /api/pdf/design`, sorted by path.
pub async fn list(storage: &Storage) -> Result<Vec<DesignFile>, AppError> {
    Ok(effective(&stored(storage, DESIGN_PREFIX).await?))
}

/// Embedded ∪ the listed live files `custom` (by design path).
pub fn effective(custom: &BTreeMap<String, Object>) -> Vec<DesignFile> {
    let mut files: BTreeMap<String, DesignFile> = embedded()
        .into_iter()
        .map(|(path, size)| {
            let file = DesignFile {
                path: path.clone(),
                source: Source::Default,
                size,
            };
            (path, file)
        })
        .collect();
    for (path, o) in custom {
        let file = DesignFile {
            path: path.clone(),
            source: Source::Custom,
            size: o.version.size,
        };
        files.insert(path.clone(), file);
    }
    files.into_values().collect()
}

fn is_font(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("fonts/") && (lower.ends_with(".ttf") || lower.ends_with(".otf"))
}

/// A loaded design, ready to go into a render request.
pub struct Design {
    pub template: String,
    /// Every other file, keyed by relative path (the template and `qr.svg`
    /// excluded). Embedded files borrow the binary's static data.
    pub files: BTreeMap<String, Bytes>,
    /// The known digests of `files` (the embedded ones).
    pub digests: BTreeMap<String, Digest>,
}

impl Design {
    /// Font keys for the request, sorted by path.
    pub fn fonts(&self) -> Vec<String> {
        self.files.keys().filter(|k| is_font(k)).cloned().collect()
    }

    /// `logo.svg`, else `logo.png`.
    pub fn logo(&self) -> Option<String> {
        ["logo.svg", "logo.png"]
            .into_iter()
            .find(|k| self.files.contains_key(*k))
            .map(str::to_string)
    }

    pub fn signature(&self) -> Option<String> {
        self.files
            .contains_key("signature.png")
            .then(|| "signature.png".to_string())
    }
}

fn too_large(path: &str, size: u64) -> Result<(), AppError> {
    if size > MAX_FILE_BYTES {
        return Err(AppError::PdfRenderFailed(format!(
            "design file {path} is larger than 10 MB"
        )));
    }
    Ok(())
}

/// Read the effective live set. A file over 10 MB fails the render
/// (`pdf_render_failed`, checked before any download); an unreachable
/// storage is `storage_unavailable`.
pub async fn load(storage: &Storage, cache: &Cache, scope: &str) -> Result<Design, AppError> {
    load_layers(storage, cache, scope, Layers::LIVE).await
}

/// Read embedded ∪ `layers`. Only live files go through the cache, and only
/// a load of the live set evicts its entries.
pub async fn load_layers(
    storage: &Storage,
    cache: &Cache,
    scope: &str,
    layers: Layers<'_>,
) -> Result<Design, AppError> {
    let custom = if layers.is_live() {
        let custom = match cache.listing(scope) {
            Ok(listed) => listed,
            Err(epoch) => {
                let listed = layers.overrides(storage).await?;
                cache.store_listing(scope, epoch, listed.clone());
                listed
            }
        };
        let keys: Vec<String> = custom.values().map(|o| scoped_key(scope, &o.key)).collect();
        cache.retain(scope, &keys);
        custom
    } else {
        layers.overrides(storage).await?
    };
    let mut files = BTreeMap::new();
    let mut digests = BTreeMap::new();
    for (path, e) in embedded_files() {
        if path == QR_IMAGE || custom.contains_key(path) {
            continue;
        }
        too_large(path, e.bytes.len() as u64)?;
        files.insert(path.clone(), e.bytes.clone());
        digests.insert(path.clone(), e.digest.clone());
    }
    for (path, o) in &custom {
        if path == QR_IMAGE {
            continue;
        }
        too_large(path, o.version.size)?;
        // Only live files are cached: a draft or version is read for one
        // check and would otherwise linger until the next live render.
        let bytes = if design_path(&o.key).is_some() {
            cache.fetch(storage, scope, o).await?
        } else {
            storage.get(&o.key).await?
        };
        files.insert(path.clone(), bytes);
    }
    let template = files
        .remove(TEMPLATE)
        .context("the design has no invoice.typ")?;
    digests.remove(TEMPLATE);
    let template = String::from_utf8(template.to_vec())
        .map_err(|_| AppError::PdfRenderFailed(format!("{TEMPLATE} is not valid UTF-8")))?;
    Ok(Design {
        template,
        files,
        digests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_paths() {
        assert_eq!(design_path("design/logo.png"), Some("logo.png"));
        assert_eq!(design_path("design/fonts/a.ttf"), Some("fonts/a.ttf"));
        assert_eq!(design_path("design/.git/config"), None);
        assert_eq!(design_path("design/fonts/.DS_Store"), None);
        assert_eq!(design_path("designer/x"), None);
        assert_eq!(design_path("documents/2026/x.pdf"), None);
        assert_eq!(key("logo.svg"), "design/logo.svg");
    }

    #[test]
    fn default_design_is_embedded() {
        let files = embedded();
        assert!(files.contains_key(TEMPLATE));
        assert!(files.contains_key("fonts/OFL.txt"));
        let fonts: Vec<_> = files.keys().filter(|k| is_font(k)).collect();
        assert!(fonts.len() >= 3, "{fonts:?}");
        assert!(fonts.iter().all(|f| f.starts_with("fonts/Inter-")));
    }

    #[test]
    fn embedded_digests_are_computed_once_and_match() {
        let e = &embedded_files()[TEMPLATE];
        assert_eq!(e.digest, mdcast_client::sha256(&e.bytes));
        let again = &embedded_files()[TEMPLATE];
        assert!(std::ptr::eq(e, again), "one table per process");
        assert_eq!(embedded()[TEMPLATE], e.bytes.len() as u64);
    }

    #[test]
    fn logo_prefers_svg() {
        let mut design = Design {
            template: String::new(),
            files: BTreeMap::new(),
            digests: BTreeMap::new(),
        };
        assert_eq!((design.logo(), design.signature()), (None, None));
        design.files.insert("logo.png".into(), Bytes::new());
        assert_eq!(design.logo().as_deref(), Some("logo.png"));
        design.files.insert("logo.svg".into(), Bytes::new());
        design.files.insert("signature.png".into(), Bytes::new());
        assert_eq!(design.logo().as_deref(), Some("logo.svg"));
        assert_eq!(design.signature().as_deref(), Some("signature.png"));
    }

    #[test]
    fn size_limit() {
        assert!(too_large("a", MAX_FILE_BYTES).is_ok());
        assert!(matches!(
            too_large("logo.png", MAX_FILE_BYTES + 1),
            Err(AppError::PdfRenderFailed(m)) if m.contains("logo.png")
        ));
    }
}
