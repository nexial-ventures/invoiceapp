//! The overview's database side: the documents of a tax-date range in CZK
//! and the open (unpaid) documents, read inside the caller's snapshot.

use std::collections::HashMap;

use anyhow::Context as _;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
};
use uuid::Uuid;

use super::aggregate::Doc;
use super::open::OpenRow;
use crate::accounting::doc::shown_number_of;
use crate::document::doc_type::DocType;
use crate::document::entity::document::{Column, Entity};
use crate::document::entity::vat_recap;
use crate::document::line::{Direction, Status, VatMode};
use crate::error::AppError;
use crate::export::load::CHUNK;
use crate::export::period::{TAX_DATE, tax_date_between};
use crate::export::source::{czk_recap_of, czk_total, to_czk};
use crate::space::SpaceId;

#[derive(Debug, FromQueryResult)]
struct DocRow {
    id: Uuid,
    direction: Direction,
    doc_type: DocType,
    vat_mode: VatMode,
    vat_deductible: bool,
    currency: String,
    exchange_rate: Option<Decimal>,
    total: Decimal,
    rounding: Decimal,
    total_czk: Option<Decimal>,
    category_id: Option<Uuid>,
    tax_date: NaiveDate,
}

async fn recaps<C: ConnectionTrait>(
    db: &C,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<vat_recap::Model>>, AppError> {
    let mut by: HashMap<Uuid, Vec<vat_recap::Model>> = HashMap::new();
    for chunk in ids.chunks(CHUNK) {
        let rows = vat_recap::Entity::find()
            .filter(vat_recap::Column::DocumentId.is_in(chunk.to_vec()))
            .all(db)
            .await?;
        for r in rows {
            by.entry(r.document_id).or_default().push(r);
        }
    }
    Ok(by)
}

/// The issued-status, non-proforma documents whose tax date is in
/// `from..=to` (the accountant export's set), in CZK.
pub async fn docs<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Doc>, AppError> {
    let rows = Entity::find()
        .filter(Column::SpaceId.eq(space))
        .filter(Column::Status.eq(Status::Issued))
        .filter(Column::DocType.ne(DocType::Proforma))
        .filter(tax_date_between(from, to))
        .select_only()
        .columns([
            Column::Id,
            Column::Direction,
            Column::DocType,
            Column::VatMode,
            Column::VatDeductible,
            Column::Currency,
            Column::ExchangeRate,
            Column::Total,
            Column::Rounding,
            Column::TotalCzk,
            Column::CategoryId,
        ])
        .column_as(Expr::cust(TAX_DATE), "tax_date")
        .into_model::<DocRow>()
        .all(db)
        .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let recaps = recaps(db, &ids).await?;
    rows.into_iter()
        .map(|r| {
            let recap = recaps.get(&r.id).map_or(&[][..], Vec::as_slice);
            Ok(Doc {
                direction: r.direction,
                doc_type: r.doc_type,
                vat_mode: r.vat_mode,
                vat_deductible: r.vat_deductible,
                tax_date: r.tax_date,
                category_id: r.category_id,
                recap: czk_recap_of(&r.currency, r.exchange_rate, recap)
                    .with_context(|| format!("document {}", r.id))?,
                total: czk_total(&r.currency, r.total, r.rounding, r.total_czk)
                    .with_context(|| format!("foreign document {} without total_czk", r.id))?,
            })
        })
        .collect()
}

#[derive(Debug, FromQueryResult)]
struct OpenDbRow {
    id: Uuid,
    direction: Direction,
    doc_type: DocType,
    number: Option<String>,
    supplier_number: Option<String>,
    counterparty: Option<String>,
    due_date: Option<NaiveDate>,
    currency: String,
    exchange_rate: Option<Decimal>,
    payable: Decimal,
    paid: Decimal,
}

/// A proforma no non-cancelled invoice settles yet.
const UNSETTLED: &str = "(doc_type <> 'proforma' OR NOT EXISTS (\
    SELECT 1 FROM documents s WHERE s.space_id = documents.space_id \
    AND s.related_document_id = documents.id AND s.doc_type = 'invoice' \
    AND s.status <> 'cancelled'))";

const COUNTERPARTY: &str = "CASE WHEN direction = 'issued' \
    THEN customer_snapshot ->> 'name' ELSE supplier_snapshot ->> 'name' END";

/// Issued-status documents with `paid < payable`, today: every type that
/// takes payments (not a DDPP), proformas only while unsettled.
pub async fn open<C: ConnectionTrait>(db: &C, space: SpaceId) -> Result<Vec<OpenRow>, AppError> {
    let rows = Entity::find()
        .filter(Column::SpaceId.eq(space))
        .filter(Column::Status.eq(Status::Issued))
        .filter(Column::DocType.ne(DocType::AdvanceTaxDoc))
        .filter(Expr::col(Column::Paid).lt(Expr::col(Column::Payable)))
        .filter(Expr::cust(UNSETTLED))
        .select_only()
        .columns([
            Column::Id,
            Column::Direction,
            Column::DocType,
            Column::Number,
            Column::SupplierNumber,
            Column::DueDate,
            Column::Currency,
            Column::ExchangeRate,
            Column::Payable,
            Column::Paid,
        ])
        .column_as(Expr::cust(COUNTERPARTY), "counterparty")
        .into_model::<OpenDbRow>()
        .all(db)
        .await?;
    rows.into_iter()
        .map(|r| {
            let remaining = to_czk(r.payable - r.paid, &r.currency, r.exchange_rate)
                .with_context(|| format!("foreign document {} without a rate", r.id))?;
            Ok(OpenRow {
                id: r.id,
                direction: r.direction,
                doc_type: r.doc_type,
                number: shown_number_of(
                    r.direction,
                    r.number.as_deref(),
                    r.supplier_number.as_deref(),
                )
                .map(str::to_string),
                counterparty: r.counterparty,
                due_date: r.due_date,
                remaining,
            })
        })
        .collect()
}
