//! The draft preview and the sample renders behind activation checks.

use axum::extract::State;
use axum::response::Response;

use super::{DRAFT_PREFIX, repo};
use crate::app::AppState;
use crate::auth::Manage;
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiQuery;
use crate::pdf::PdfService;
use crate::pdf::archive::PdfBody;
use crate::pdf::design::Layers;
use crate::pdf::format::Locale;
use crate::pdf::handlers::{PreviewQuery, pdf_response, preview_locale};
use crate::pdf::payload;
use crate::pdf::preview::Sample;
use crate::settings::entity::{bank_account, company as company_entity};
use crate::settings::repo::company;
use crate::space::SpaceId;
use crate::storage::DESIGN_PREFIX;
use crate::time::today;

/// What the sample is built from, read once per request.
pub(super) struct SampleSource {
    pub company: company_entity::Model,
    bank: Option<bank_account::Model>,
}

impl SampleSource {
    pub async fn load(state: &AppState, space: SpaceId) -> Result<Self, AppError> {
        let company = company::get(&state.db, space).await?;
        let bank = Sample::bank(&state.db, &company).await?;
        Ok(Self { company, bank })
    }

    /// Render the sample in `locale` with `layers` over the default.
    pub async fn render(
        &self,
        pdf: &PdfService,
        locale: Locale,
        layers: Layers<'_>,
    ) -> Result<Vec<u8>, AppError> {
        let sample = Sample::new(&self.company, self.bank.as_ref(), locale, today())?;
        pdf.render_with(payload::build(&sample.input())?, layers)
            .await
    }
}

#[utoipa::path(
    get,
    path = "/api/pdf/design/draft/preview",
    tag = "pdf",
    security(("cookie" = []), ("bearer" = [])),
    params(PreviewQuery),
    responses(
        (status = 200, description = "The sample invoice with embedded ∪ live ∪ draft (minus deletions)", content_type = "application/pdf"),
        (status = 422, description = "`locale`: `invalid`", body = ErrorBody),
        (status = 502, description = "`pdf_render_failed` (+ `detail`)", body = ErrorBody),
        (status = 503, description = "`pdf_unavailable`, `storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn preview(
    State(state): State<AppState>,
    access: Manage,
    ApiQuery(q): ApiQuery<PreviewQuery>,
) -> Result<Response, AppError> {
    let space = access.space();
    let source = SampleSource::load(&state, space).await?;
    let locale = preview_locale(&source.company, q.locale.as_deref())?;
    let pdf = state.pdf.space(space)?;
    let deleted = repo::draft(&state.db, space)
        .await?
        .map(|r| r.deleted)
        .unwrap_or_default();
    let layers = Layers {
        base: DESIGN_PREFIX,
        deleted: &deleted,
        top: Some(DRAFT_PREFIX),
    };
    let bytes = source.render(&pdf, locale, layers).await?;
    Ok(pdf_response(
        PdfBody::Rendered(bytes.into()),
        "draft-preview",
        false,
    ))
}
