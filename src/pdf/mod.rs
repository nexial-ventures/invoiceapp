//! PDF rendering through the mdcast service: design files, the `/data.json`
//! payload, the SPAYD QR code, and the archive of issued documents.

pub mod amounts;
pub mod archive;
pub mod client;
pub mod design;
pub mod design_cache;
pub mod editor;
pub mod format;
pub mod handlers;
pub mod labels;
pub mod limits;
pub mod payload;
pub mod preview;
pub mod reference;
pub mod source;
pub mod spayd;
#[cfg(test)]
mod test_fixtures;

use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use axum::routing::get;
use mdcast_client::{AssetBundle, Client, request};

use crate::app::AppState;
use crate::error::AppError;
use crate::space::SpaceId;
use crate::storage::Storage;
use limits::{RenderKind, RenderLimits};
use payload::Payload;

pub const DEFAULT_MDCAST_URL: &str = "https://mdcast.nexial.cz";
const MDCAST_TIMEOUT: Duration = Duration::from_secs(60);

/// The instance-wide PDF service: mdcast client, the root storage and the
/// shared design cache. It renders and stores nothing itself — every route
/// works through [`PdfRoot::space`], so no key can land outside a space.
#[derive(Clone)]
pub struct PdfRoot {
    client: Client,
    storage: Storage,
    design_cache: design::Cache,
    limits: RenderLimits,
}

impl PdfRoot {
    /// Builds the client (no network).
    pub fn new(url: &str, token: Option<&str>, storage: Storage) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(MDCAST_TIMEOUT)
            .build()
            .context("build mdcast HTTP client")?;
        // mdcast-client insists on a token; a server without a bearer gate
        // ignores the header, so a placeholder is safe.
        let token = token
            .filter(|t| !t.trim().is_empty())
            .unwrap_or("unauthenticated");
        let client = Client::builder(url)
            .token(token)
            .http_client(http)
            .build()
            .context("INVOICE__MDCAST_URL must start with http:// or https://")?;
        Ok(Self {
            client,
            storage,
            design_cache: design::Cache::default(),
            limits: RenderLimits::default(),
        })
    }

    /// The service of one space: storage keys under `spaces/{id}/`, the
    /// design overlay read from `spaces/{id}/design/`.
    pub fn space(&self, space: SpaceId) -> Result<PdfService, AppError> {
        let prefix = space.storage_prefix();
        Ok(PdfService {
            client: self.client.clone(),
            storage: self.storage.scoped(&prefix)?,
            design_cache: self.design_cache.clone(),
            limits: self.limits.clone(),
            kind: RenderKind::Archive,
            space,
        })
    }

    /// Replace the render limits (`INVOICE__MDCAST_CONCURRENCY`; tests use
    /// shorter waits).
    pub fn with_render_limits(mut self, limits: RenderLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Bound the design cache to `mb` MiB (`INVOICE__DESIGN_CACHE_MB`;
    /// [`design_cache::DEFAULT_LIMIT_MB`] otherwise).
    pub fn with_design_cache_mb(mut self, mb: u32) -> Self {
        self.design_cache = design::Cache::with_limit(u64::from(mb) * 1024 * 1024);
        self
    }

    /// The unscoped storage (space delete removes a whole prefix).
    pub fn root_storage(&self) -> &Storage {
        &self.storage
    }

    pub fn design_cache(&self) -> &design::Cache {
        &self.design_cache
    }
}

/// The mdcast client plus the storage of one space: its design overlay,
/// archives and originals.
#[derive(Clone)]
pub struct PdfService {
    client: Client,
    storage: Storage,
    design_cache: design::Cache,
    limits: RenderLimits,
    kind: RenderKind,
    space: SpaceId,
}

impl PdfService {
    /// The space this service reads and writes.
    pub fn space_id(&self) -> SpaceId {
        self.space
    }

    /// The same service rendering previews: the per-space preview limits
    /// and the shorter timeout apply ([`limits`]).
    pub fn preview(&self) -> Self {
        Self {
            kind: RenderKind::Preview,
            ..self.clone()
        }
    }

    /// Archives, originals and the design overrides.
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// Render `payload` with the current live design (listing cached for
    /// [`design_cache::LISTING_TTL`]).
    pub async fn render(&self, payload: Payload) -> Result<Vec<u8>, AppError> {
        self.render_with(payload, design::Layers::LIVE).await
    }

    /// Forget the cached design files of this space (activation, restore,
    /// reset).
    pub fn invalidate_design(&self) {
        self.design_cache.invalidate(&self.space.storage_prefix());
    }

    /// Render `payload` with the embedded default overridden by `layers`
    /// (a draft or version check; [`render`](Self::render) uses the live set).
    pub async fn render_with(
        &self,
        mut payload: Payload,
        layers: design::Layers<'_>,
    ) -> Result<Vec<u8>, AppError> {
        let slot = match self.kind {
            RenderKind::Preview => Some(self.limits.admit_preview(self.space)?),
            RenderKind::Archive => None,
        };
        let design = design::load_layers(
            &self.storage,
            &self.design_cache,
            &self.space.storage_prefix(),
            layers,
        )
        .await?;
        payload.assets = payload::Assets {
            logo: design.logo(),
            signature: design.signature(),
        };
        let fonts = design.fonts();
        let mut bundle = AssetBundle::new();
        for (key, bytes) in design.files {
            match design.digests.get(&key) {
                // Embedded: digest precomputed, bytes only sent if mdcast
                // asks for them.
                Some(digest) => {
                    bundle.insert_digest(key, digest.clone(), move || {
                        std::future::ready(Ok::<_, std::convert::Infallible>(bytes.clone()))
                    });
                }
                None => {
                    bundle.insert(key, bytes);
                }
            }
        }
        if let Some(s) = &payload.spayd {
            bundle.insert(design::QR_IMAGE, spayd::qr_svg(s)?.into_bytes());
        }
        let data = serde_json::to_string(&payload).context("serialize PDF payload")?;
        let mut req = request::template(design.template, data);
        req.fonts = fonts;
        let _permit = self.limits.permit().await.inspect_err(|_| {
            if let Some(slot) = &slot {
                slot.refund();
            }
        })?;
        let render = self.client.render_template(req, &bundle);
        let artifact = match self.limits.timeout(self.kind) {
            Some(limit) => tokio::time::timeout(limit, render).await.map_err(|_| {
                AppError::PdfUnavailable(format!("preview render took over {limit:?}"))
            })?,
            None => render.await,
        }
        .map_err(client::map_error)?;
        Ok(artifact.bytes.to_vec())
    }
}

/// Routes relative to `/api/pdf`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/preview", get(handlers::preview))
        .merge(editor::router())
}
