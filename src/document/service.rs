//! Document operations shared by the REST handlers and the MCP tools: they
//! load what validation needs, validate and write through the repos.

use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::app::AppState;
use crate::document::entity::document;
use crate::document::line::{MAX_AMOUNT, Status};
use crate::document::repo::payments::{self as payments_repo, NewPayment, WithAdvance};
use crate::document::repo::{
    advance_sources, context, ddpp_correction, lifecycle, query, view, write,
};
use crate::document::wire::compute_input::{ComputeCtx, ComputeInput};
use crate::document::wire::dto::{
    Computed, Document, DocumentList, ListQuery, Payment, PaymentInput, lines_out,
};
use crate::document::wire::fields::exchange_rate;
use crate::document::wire::input::DocumentInput;
use crate::error::{AppError, FieldErrors};
use crate::num_parse::decimal;
use crate::pdf::archive;
use crate::settings::repo::company;
use crate::space::SpaceId;
use crate::time::today;
use crate::validation as v;

/// Re-read a document and build its response.
pub async fn fetch(state: &AppState, space: SpaceId, id: Uuid) -> Result<Document, AppError> {
    view::document(query::load(&state.db, space, id).await?, today())
}

/// One page of the document list; `term` / `limit` / `offset` already normalized.
pub async fn list_page(
    state: &AppState,
    space: SpaceId,
    query: &ListQuery,
    term: Option<&str>,
    limit: u64,
    offset: u64,
) -> Result<DocumentList, AppError> {
    let today = today();
    let (docs, total) = query::list(&state.db, space, query, term, limit, offset, today).await?;
    Ok(DocumentList {
        items: view::summaries(&state.db, docs, today).await?,
        total,
    })
}

/// Create an issued-direction draft; omitted fields get the contact / company defaults.
pub async fn create_draft(
    state: &AppState,
    space: SpaceId,
    input: DocumentInput,
) -> Result<Uuid, AppError> {
    let ctx = context::load(&state.db, space, &input, today(), None).await?;
    let (mut data, evaluated) = input.validate(&ctx)?;
    context::resolve_bank(&state.db, space, &mut data, true).await?;
    write::create(&state.db, space, data, evaluated.totals).await
}

/// Replace every field of an issued-direction draft (`document_locked` otherwise).
pub async fn update_draft(
    state: &AppState,
    space: SpaceId,
    doc: &document::Model,
    input: DocumentInput,
) -> Result<(), AppError> {
    if view::status(doc)? != Status::Draft {
        return Err(AppError::DocumentLocked);
    }
    let ctx = context::load(&state.db, space, &input, today(), Some(doc)).await?;
    let (mut data, evaluated) = input.validate(&ctx)?;
    context::resolve_bank(&state.db, space, &mut data, false).await?;
    write::update(&state.db, space, doc.id, data, evaluated.totals).await
}

/// Live lines + totals of an unsaved document, no writes.
pub async fn compute_totals(
    state: &AppState,
    space: SpaceId,
    input: ComputeInput,
) -> Result<Computed, AppError> {
    let company = company::get(&state.db, space).await?;
    let existing = match input.document_id {
        Some(id) => match query::find(&state.db, space, id).await {
            Ok(doc) => Some(context::existing(&doc)?),
            Err(AppError::NotFound) => None,
            Err(e) => return Err(e),
        },
        None => None,
    };
    let exact = ddpp_correction::basis_for(&state.db, existing.as_ref()).await?;
    let ctx = ComputeCtx {
        vat_payer: company.vat_payer,
        default_rate: context::default_vat_rate(&state.db, space).await?,
        default_locale: company.default_locale,
        existing,
        advances: advance_sources::load(&state.db, space, &input.advance_ids()).await?,
        exact,
    };
    let (lines, evaluated) = input.validate(&ctx)?;
    Ok(Computed {
        lines: lines_out(&lines, &evaluated.lines),
        totals: evaluated.totals.into(),
    })
}

/// Set `sentAt` of an issued document (default now).
pub async fn mark_sent_at(
    state: &AppState,
    space: SpaceId,
    id: Uuid,
    sent_at: Option<DateTime<FixedOffset>>,
) -> Result<(), AppError> {
    let sent_at = sent_at.unwrap_or_else(|| chrono::Utc::now().fixed_offset());
    lifecycle::mark_sent(&state.db, space, id, sent_at).await
}

impl From<WithAdvance> for Payment {
    fn from((m, advance): WithAdvance) -> Self {
        Self {
            id: m.id,
            date: m.date,
            amount: crate::document::compute::round2(m.amount),
            note: m.note,
            advance_document_id: advance,
            created_at: m.created_at,
        }
    }
}

impl PaymentInput {
    pub fn validate(self) -> Result<NewPayment, AppError> {
        let mut e = FieldErrors::new();
        let date = e.check("date", self.date.ok_or("required"));
        let amount = e.check(
            "amount",
            decimal(&self.amount, 2, MAX_AMOUNT).and_then(|a| {
                if a > Decimal::ZERO {
                    Ok(a)
                } else {
                    Err("invalid")
                }
            }),
        );
        let note = e
            .check("note", v::opt_text(self.note.as_deref(), 500))
            .flatten();
        let exchange_rate = e
            .check("exchangeRate", exchange_rate(self.exchange_rate.as_deref()))
            .flatten();
        e.into_result()?;
        Ok(NewPayment {
            date: date.unwrap_or_default(),
            amount: amount.unwrap_or_default(),
            note,
            exchange_rate,
        })
    }
}

/// Record a payment; on a VAT payer's proforma this issues its DDPP, whose
/// PDF is archived in the background (never failing the payment).
pub async fn add_payment(
    state: &AppState,
    space: SpaceId,
    id: Uuid,
    input: PaymentInput,
) -> Result<Payment, AppError> {
    let row =
        payments_repo::create(&state.db, &state.cnb, space, id, input.validate()?, today()).await?;
    if let Some(ddpp) = row.1 {
        archive::spawn_archive_ddpp(state.db.clone(), state.pdf.space(space)?, ddpp);
    }
    Ok(row.into())
}
