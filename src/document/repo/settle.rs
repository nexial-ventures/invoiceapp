//! Final invoice from a proforma: a draft invoice copying the proforma and
//! deducting its advances (payer: its DDPPs; non-payer form: the paid amount).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    TransactionTrait,
};
use uuid::Uuid;

use super::{advance_sources, context, query, view, write};
use crate::document::advance::AdvanceCtx;
use crate::document::entity::document;
use crate::document::handlers::input::{self, DocumentData};
use crate::document::line::{AdvanceData, LineData, PaymentMethod, Status};
use crate::error::AppError;
use crate::settings::doc_type::{DocType, ISSUED};
use crate::space::SpaceId;

/// Whether a non-cancelled invoice settles the proforma `id`.
async fn settled<C: ConnectionTrait>(db: &C, id: Uuid) -> Result<bool, AppError> {
    Ok(document::Entity::find()
        .filter(document::Column::RelatedDocumentId.eq(id))
        .filter(document::Column::DocType.eq(DocType::Invoice.as_str()))
        .filter(document::Column::Status.ne(Status::Cancelled.as_str()))
        .one(db)
        .await?
        .is_some())
}

fn settleable(doc: &document::Model) -> Result<(), AppError> {
    if doc.doc_type != DocType::Proforma.as_str()
        || doc.direction != ISSUED
        || view::status(doc)? != Status::Issued
    {
        return Err(AppError::InvalidState);
    }
    Ok(())
}

/// The documents to deduct: the proforma's issued DDPPs (oldest first), or
/// the proforma itself when its payments issued none and something is paid.
async fn advances(db: &DatabaseConnection, p: &document::Model) -> Result<Vec<Uuid>, AppError> {
    if super::ddpp::needed(p) {
        return Ok(document::Entity::find()
            .filter(document::Column::RelatedDocumentId.eq(p.id))
            .filter(document::Column::DocType.eq(DocType::AdvanceTaxDoc.as_str()))
            .filter(document::Column::Status.eq(Status::Issued.as_str()))
            .order_by_asc(document::Column::IssueDate)
            .order_by_asc(document::Column::NumberSeq)
            .all(db)
            .await?
            .into_iter()
            .map(|d| d.id)
            .collect());
    }
    Ok(if p.paid > Decimal::ZERO {
        vec![p.id]
    } else {
        Vec::new()
    })
}

pub async fn settle(
    db: &DatabaseConnection,
    space: SpaceId,
    proforma_id: Uuid,
    today: NaiveDate,
) -> Result<Uuid, AppError> {
    let p = query::find(db, space, proforma_id).await?;
    settleable(&p)?;
    if settled(db, proforma_id).await? {
        return Err(AppError::InvalidState);
    }
    let ids = advances(db, &p).await?;
    let sources = advance_sources::load(db, space, &ids).await?;
    let mut lines = query::load_lines(db, proforma_id).await?;
    // A DDPP the user already put on another invoice stays there; a fully
    // corrected one has nothing left to deduct.
    lines.extend(
        ids.iter()
            .filter(|id| {
                sources
                    .get(id)
                    .is_some_and(|s| s.referenced_by.is_empty() && !s.recap.is_empty())
            })
            .map(|id| {
                LineData::Advance(AdvanceData {
                    document_id: *id,
                    description: String::new(),
                    recap: Vec::new(),
                })
            }),
    );
    let due_date = context::due_date(db, space, p.contact_id, today).await?;
    let mut data = DocumentData {
        doc_type: DocType::Invoice,
        related_document_id: Some(p.id),
        correction_reason: None,
        contact_id: p.contact_id,
        issue_date: today,
        tax_point_date: Some(today),
        due_date,
        currency: p.currency.clone(),
        // The invoice's rate is fixed at its own issue (tax point date).
        exchange_rate: None,
        locale: p.locale.clone(),
        vat_mode: input::parse_vat_mode(&p.vat_mode)
            .map_err(|_| anyhow::anyhow!("proforma {proforma_id} has unknown vat mode"))?,
        bank_account_id: p.bank_account_id,
        payment_method: PaymentMethod::parse(&p.payment_method)
            .unwrap_or(PaymentMethod::BankTransfer),
        variable_symbol: None,
        constant_symbol: p.constant_symbol.clone(),
        order_ref: p.order_ref.clone(),
        header_note: p.header_note.clone(),
        footer_note: p.footer_note.clone(),
        internal_note: None,
        rounding_mode: view::rounding_mode(&p)?,
        lines: Vec::new(),
        imported: false,
        number: None,
        meta: Default::default(),
    };
    let adv = AdvanceCtx {
        doc_type: DocType::Invoice,
        vat_mode: data.vat_mode,
        document_id: None,
        related_document_id: Some(p.id),
        contact_id: p.contact_id,
        currency: &p.currency,
        locale: &p.locale,
        sources: &sources,
    };
    let totals = input::evaluate(&mut lines, data.params(), &adv, None)?.totals;
    data.lines = lines;
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let p = query::lock(txn, space, proforma_id).await?;
                settleable(&p)?;
                if settled(txn, proforma_id).await? {
                    return Err(AppError::InvalidState);
                }
                write::create_in(txn, space, data, totals).await
            })
        })
        .await?)
}
