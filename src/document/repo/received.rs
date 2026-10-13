//! Received documents: no draft — created recorded (`issued`) with an
//! internal number, replaced and deleted at any time.

use chrono::Datelike;
use sea_orm::{
    ActiveModelTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, Set, TransactionTrait,
};
use uuid::Uuid;

use super::issue::Rate;
use super::{NewDocument, insert, lines, query, write};
use crate::document::compute::Totals;
use crate::document::entity::document::{self, ActiveModel, Entity};
use crate::document::line::Direction;
use crate::document::line::{PaymentMethod, RoundingMode, Status};
use crate::document::model::ReceivedData;
use crate::error::{AppError, number_violation};
use crate::settings::repo::number_series;
use crate::space::SpaceId;

/// Everything a received document's row is written from.
pub struct Record {
    pub data: ReceivedData,
    pub totals: Totals,
    pub rate: Rate,
    /// The supplier (contact) snapshot, refreshed on every save.
    pub supplier: serde_json::Value,
}

/// Informational: the entered rounding amount is kept as given.
fn rounding_mode(t: &Totals) -> RoundingMode {
    RoundingMode::derive(t.total, t.rounding)
}

fn apply(row: &mut ActiveModel, r: Record) -> Totals {
    let d = r.data;
    row.contact_id = Set(Some(d.contact_id));
    row.supplier_number = Set(Some(d.supplier_number));
    row.issue_date = Set(d.issue_date);
    row.tax_point_date = Set(d.tax_point_date);
    row.received_date = Set(Some(d.received_date));
    row.due_date = Set(d.due_date);
    row.currency = Set(d.currency);
    row.exchange_rate = Set(r.rate.rate);
    row.exchange_rate_date = Set(r.rate.date);
    row.exchange_rate_source = Set(r.rate.source.map(str::to_string));
    row.vat_mode = Set(d.vat_mode);
    row.vat_deductible = Set(d.vat_deductible);
    row.variable_symbol = Set(d.variable_symbol);
    row.constant_symbol = Set(d.constant_symbol);
    row.supplier_account = Set(d.supplier_account);
    row.related_document_id = Set(d.related_document_id);
    row.category_id = Set(d.meta.category_id);
    row.custom_fields = Set(serde_json::Value::Object(d.meta.custom_fields));
    row.internal_note = Set(d.internal_note);
    row.supplier_snapshot = Set(Some(r.supplier));
    write::apply_totals(row, &r.totals);
    row.rounding_mode = Set(rounding_mode(&r.totals));
    row.updated_at = Set(chrono::Utc::now().into());
    r.totals
}

/// Allocates the internal number from the doc type's received series for
/// the `receivedDate` year, in the same transaction.
pub async fn create(
    db: &DatabaseConnection,
    space: SpaceId,
    r: Record,
    locale: String,
) -> Result<Uuid, AppError> {
    Ok(db
        .transaction(|txn| Box::pin(create_in(txn, space, r, locale)))
        .await?)
}

async fn create_in(
    txn: &DatabaseTransaction,
    space: SpaceId,
    r: Record,
    locale: String,
) -> Result<Uuid, AppError> {
    let id = Uuid::new_v4();
    let year = r.data.received_date.year();
    let doc_type = r.data.doc_type;
    let (number, seq) =
        number_series::allocate_number(txn, space, doc_type.series(Direction::Received), year)
            .await?;
    let now = chrono::Utc::now().into();
    let (d, totals) = (r.data, r.totals);
    let row = NewDocument {
        id,
        space_id: space.uuid(),
        direction: Direction::Received,
        doc_type,
        status: Status::Issued,
        number: Some(number),
        number_year: Some(year),
        number_seq: Some(seq),
        number_code: None,
        imported: false,
        contact_id: Some(d.contact_id),
        issue_date: d.issue_date,
        tax_point_date: d.tax_point_date,
        due_date: d.due_date,
        currency: d.currency,
        exchange_rate: r.rate.rate,
        exchange_rate_date: r.rate.date,
        exchange_rate_source: r.rate.source.map(str::to_string),
        locale,
        vat_mode: d.vat_mode,
        bank_account_id: None,
        payment_method: PaymentMethod::BankTransfer,
        variable_symbol: d.variable_symbol,
        constant_symbol: d.constant_symbol,
        order_ref: None,
        header_note: None,
        footer_note: None,
        internal_note: d.internal_note,
        rounding_mode: rounding_mode(&totals),
        supplier_snapshot: Some(r.supplier),
        customer_snapshot: None,
        bank_snapshot: None,
        totals: totals.clone(),
        related_document_id: d.related_document_id,
        payment_id: None,
        correction_reason: None,
        supplier_number: Some(d.supplier_number),
        received_date: Some(d.received_date),
        vat_deductible: d.vat_deductible,
        supplier_account: d.supplier_account,
        category_id: d.meta.category_id,
        custom_fields: serde_json::Value::Object(d.meta.custom_fields),
        original_path: None,
        original_sha256: None,
        original_size: None,
        original_uploaded_at: None,
        created_at: now,
        updated_at: now,
    };
    insert(txn, row)
        .await
        .map_err(|e| number_violation(e, || AppError::NumberTaken))?;
    lines::replace_recap(txn, id, &totals).await?;
    Ok(id)
}

/// Lock the row and require a received document.
async fn locked(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
) -> Result<document::Model, AppError> {
    let doc = query::lock(txn, space, id).await?;
    if doc.direction != Direction::Received {
        return Err(AppError::InvalidState);
    }
    Ok(doc)
}

/// The number never changes (also not with the `receivedDate` year).
pub async fn update(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    r: Record,
) -> Result<(), AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked(txn, space, id).await?;
                let mut row: ActiveModel = doc.into();
                let totals = apply(&mut row, r);
                row.update(txn).await?;
                lines::replace_recap(txn, id, &totals).await
            })
        })
        .await?)
}

/// Payments cascade; returns the original's path for the caller to remove
/// after the commit.
pub async fn delete(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
) -> Result<Option<String>, AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked(txn, space, id).await?;
                Entity::delete_by_id(id).exec(txn).await?;
                Ok(doc.original_path)
            })
        })
        .await?)
}
