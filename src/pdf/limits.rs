//! Render limits (`docs/api/pdf.md`, Render limits): an instance-wide cap on
//! concurrent mdcast renders, and per-space limits on previews (the live
//! sample, the design draft preview, a draft document's PDF) — one in flight
//! per space and [`PREVIEW_SPACE`] per hour. Issue and archive renders only
//! pass the instance-wide cap: issuing is never refused by a preview limit,
//! and previews may hold at most all but one of the slots, so one is always
//! left for issue / archive renders (with a concurrency of 2 or more).
//! A preview that times out here may still run to its end in mdcast.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::auth::ratelimit::{PREVIEW_SPACE, RateLimiter};
use crate::error::AppError;
use crate::space::SpaceId;

/// `INVOICE__MDCAST_CONCURRENCY` default.
pub const DEFAULT_CONCURRENCY: u32 = 4;
/// Longest wait for a free render slot before 503 `pdf_unavailable` (`busy`).
pub const QUEUE_WAIT: Duration = Duration::from_secs(10);
/// mdcast timeout of a preview; issue / archive renders get the client's 60 s.
pub const PREVIEW_TIMEOUT: Duration = Duration::from_secs(20);

/// What a render is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderKind {
    /// Shown and thrown away: per-space limits, the short timeout.
    Preview,
    /// Issue, archive on first download: only the global cap.
    Archive,
    /// Design activation / restore checks: the long timeout, but the
    /// preview share of the slots (never the one kept for issuing); their
    /// own `design-space` limit replaces the per-space preview limits.
    Check,
}

#[derive(Clone)]
pub struct RenderLimits {
    permits: Arc<Semaphore>,
    /// `concurrency - 1` (at least 1): taken by a preview before a slot.
    preview_permits: Arc<Semaphore>,
    queue_wait: Duration,
    preview_timeout: Duration,
    in_flight: Arc<Mutex<HashSet<SpaceId>>>,
    hourly: RateLimiter,
}

impl Default for RenderLimits {
    fn default() -> Self {
        Self::new(DEFAULT_CONCURRENCY)
    }
}

impl RenderLimits {
    /// At most `concurrency` mdcast renders at once (at least one).
    pub fn new(concurrency: u32) -> Self {
        let n = usize::try_from(concurrency.max(1))
            .unwrap_or(usize::MAX)
            .min(Semaphore::MAX_PERMITS);
        Self {
            permits: Arc::new(Semaphore::new(n)),
            preview_permits: Arc::new(Semaphore::new((n - 1).max(1))),
            queue_wait: QUEUE_WAIT,
            preview_timeout: PREVIEW_TIMEOUT,
            in_flight: Arc::default(),
            hourly: RateLimiter::default(),
        }
    }

    /// Shorter waits (tests).
    pub fn with_waits(mut self, queue_wait: Duration, preview_timeout: Duration) -> Self {
        self.queue_wait = queue_wait;
        self.preview_timeout = preview_timeout;
        self
    }

    /// Admit a preview of `space`: 429 while another one runs (`Retry-After:
    /// 1`) or when the hourly budget is used up. The slot frees on drop.
    pub fn admit_preview(&self, space: SpaceId) -> Result<PreviewSlot, AppError> {
        if !self.lock().insert(space) {
            return Err(AppError::RateLimited(1));
        }
        let slot = PreviewSlot {
            limits: self.clone(),
            space,
        };
        self.hourly
            .reserve(&[(PREVIEW_SPACE, &space.to_string())])?;
        Ok(slot)
    }

    /// A global render slot for a `kind` render, waiting at most the queue
    /// wait (then 503 busy).
    pub async fn permit(&self, kind: RenderKind) -> Result<Permit, AppError> {
        let acquire = async {
            let preview = match kind {
                RenderKind::Preview | RenderKind::Check => {
                    Some(Arc::clone(&self.preview_permits).acquire_owned().await?)
                }
                RenderKind::Archive => None,
            };
            let slot = Arc::clone(&self.permits).acquire_owned().await?;
            Ok::<_, tokio::sync::AcquireError>(Permit {
                _slot: slot,
                _preview: preview,
            })
        };
        match tokio::time::timeout(self.queue_wait, acquire).await {
            Ok(Ok(permit)) => Ok(permit),
            // The semaphores are never closed; treat it like a full queue.
            Ok(Err(_)) | Err(_) => Err(AppError::PdfBusy),
        }
    }

    /// The render timeout of `kind`; `None` = the HTTP client's own.
    pub fn timeout(&self, kind: RenderKind) -> Option<Duration> {
        match kind {
            RenderKind::Preview => Some(self.preview_timeout),
            RenderKind::Archive | RenderKind::Check => None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<SpaceId>> {
        self.in_flight.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A held render slot (and a preview's share), released on drop.
pub struct Permit {
    _slot: OwnedSemaphorePermit,
    _preview: Option<OwnedSemaphorePermit>,
}

/// The one preview a space has in flight.
pub struct PreviewSlot {
    limits: RenderLimits,
    space: SpaceId,
}

impl PreviewSlot {
    /// The render never reached mdcast (the queue was full): it does not
    /// count against the hourly budget.
    pub fn refund(&self) {
        self.limits
            .hourly
            .refund(&[(PREVIEW_SPACE, &self.space.to_string())]);
    }
}

impl Drop for PreviewSlot {
    fn drop(&mut self) {
        self.limits.lock().remove(&self.space);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn space() -> SpaceId {
        SpaceId::from_row(Uuid::new_v4())
    }

    #[test]
    fn one_preview_in_flight_per_space() {
        let limits = RenderLimits::default();
        let (a, b) = (space(), space());
        let slot = limits.admit_preview(a).expect("first");
        assert!(matches!(
            limits.admit_preview(a),
            Err(AppError::RateLimited(1))
        ));
        let other = limits
            .admit_preview(b)
            .expect("another space is independent");
        drop(slot);
        drop(limits.admit_preview(a).expect("free again"));
        drop(other);
    }

    #[test]
    fn hourly_budget_per_space() {
        let limits = RenderLimits::default();
        let a = space();
        for _ in 0..PREVIEW_SPACE.max {
            drop(limits.admit_preview(a).expect("within the budget"));
        }
        assert!(matches!(
            limits.admit_preview(a),
            Err(AppError::RateLimited(secs)) if secs > 1
        ));
        // The refused admission released its in-flight slot.
        assert!(limits.lock().is_empty());
        drop(
            limits
                .admit_preview(space())
                .expect("other spaces unaffected"),
        );
    }

    #[test]
    fn refund_gives_the_attempt_back() {
        let limits = RenderLimits::default();
        let a = space();
        for _ in 0..PREVIEW_SPACE.max + 5 {
            let slot = limits.admit_preview(a).expect("refunded every time");
            slot.refund();
        }
    }

    #[tokio::test]
    async fn global_cap_waits_then_refuses() {
        let limits = RenderLimits::new(1).with_waits(Duration::from_millis(50), PREVIEW_TIMEOUT);
        let held = limits.permit(RenderKind::Archive).await.expect("free");
        assert!(matches!(
            limits.permit(RenderKind::Archive).await,
            Err(AppError::PdfBusy)
        ));
        drop(held);
        drop(
            limits
                .permit(RenderKind::Preview)
                .await
                .expect("free again"),
        );
        assert_eq!(limits.timeout(RenderKind::Archive), None);
        assert_eq!(limits.timeout(RenderKind::Preview), Some(PREVIEW_TIMEOUT));
        assert!(RenderLimits::new(0).permits.available_permits() == 1);
    }

    #[tokio::test]
    async fn previews_leave_one_slot_for_issue() {
        let limits = RenderLimits::new(2).with_waits(Duration::from_millis(50), PREVIEW_TIMEOUT);
        let preview = limits.permit(RenderKind::Preview).await.expect("free");
        assert!(matches!(
            limits.permit(RenderKind::Preview).await,
            Err(AppError::PdfBusy)
        ));
        // A design check shares the previews' part, keeps the long timeout.
        assert!(matches!(
            limits.permit(RenderKind::Check).await,
            Err(AppError::PdfBusy)
        ));
        assert_eq!(limits.timeout(RenderKind::Check), None);
        let issue = limits.permit(RenderKind::Archive).await.expect("reserved");
        drop((preview, issue));
    }
}
