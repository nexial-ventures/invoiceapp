//! Received documents: no draft — created recorded (`issued`) with an
//! internal number, replaced and deleted at any time.

use chrono::Datelike;
use sea_orm::{
    ActiveModelTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, Set, TransactionTrait,
};
use uuid::Uuid;

use super::issue::Rate;
use super::{lines, query, write};
use crate::document::compute::Totals;
use crate::document::entity::document::{self, ActiveModel, Entity};
use crate::document::handlers::received_input::ReceivedData;
use crate::document::line::{PaymentMethod, RoundingMode, Status};
use crate::error::{AppError, number_violation};
use crate::settings::doc_type::RECEIVED;
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
    row.vat_mode = Set(d.vat_mode.as_str().to_string());
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
    // Informational: the entered rounding amount is kept as given.
    row.rounding_mode = Set(RoundingMode::derive(r.totals.total, r.totals.rounding)
        .as_str()
        .into());
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
        number_series::allocate_number(txn, space, doc_type.series(RECEIVED), year).await?;
    let mut row = ActiveModel {
        id: Set(id),
        space_id: Set(space.uuid()),
        direction: Set(RECEIVED.into()),
        doc_type: Set(doc_type.as_str().into()),
        status: Set(Status::Issued.as_str().into()),
        number: Set(Some(number)),
        number_year: Set(Some(year)),
        number_seq: Set(Some(seq)),
        imported: Set(false),
        locale: Set(locale),
        bank_account_id: Set(None),
        payment_method: Set(PaymentMethod::BankTransfer.as_str().into()),
        created_at: Set(chrono::Utc::now().into()),
        ..Default::default()
    };
    let totals = apply(&mut row, r);
    row.insert(txn)
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
    if doc.direction != RECEIVED {
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
