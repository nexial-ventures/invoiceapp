//! Draft writes: create, replace, delete. Totals are recomputed and stored on
//! every save.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    PaginatorTrait, QueryFilter, Set, TransactionTrait,
};
use uuid::Uuid;

use super::{NewDocument, advance_sources, credit, ddpp_correction, insert, lines, query};
use crate::document::compute::Totals;
use crate::document::doc_type::DocType;
use crate::document::entity::document::{ActiveModel, Column, Entity};
use crate::document::entity::document_line;
use crate::document::line::Direction;
use crate::document::line::Status;
use crate::document::model::DocumentData;
use crate::error::{AppError, number_violation};
use crate::space::SpaceId;

/// Header fields an edit sets, shared by create and update.
fn apply(row: &mut ActiveModel, d: &DocumentData) {
    row.contact_id = Set(d.contact_id);
    row.issue_date = Set(d.issue_date);
    row.tax_point_date = Set(d.tax_point_date);
    row.due_date = Set(Some(d.due_date));
    row.currency = Set(d.currency.clone());
    row.exchange_rate = Set(d.exchange_rate);
    row.exchange_rate_date = Set(None);
    row.exchange_rate_source = Set(rate_source(d));
    row.correction_reason = Set(d.correction_reason.clone());
    row.locale = Set(d.locale.clone());
    row.vat_mode = Set(d.vat_mode);
    row.bank_account_id = Set(d.bank_account_id);
    row.payment_method = Set(d.payment_method);
    row.variable_symbol = Set(d.variable_symbol.clone());
    row.constant_symbol = Set(d.constant_symbol.clone());
    row.order_ref = Set(d.order_ref.clone());
    row.header_note = Set(d.header_note.clone());
    row.footer_note = Set(d.footer_note.clone());
    row.internal_note = Set(d.internal_note.clone());
    row.rounding_mode = Set(d.rounding_mode);
    row.number = Set(d.number.clone());
    row.related_document_id = Set(d.related_document_id);
    row.category_id = Set(d.meta.category_id);
    row.custom_fields = Set(serde_json::Value::Object(d.meta.custom_fields.clone()));
}

/// A native correction's rate is copied from its original, never entered.
fn rate_source(d: &DocumentData) -> Option<String> {
    let source = if d.doc_type.is_correction() && !d.imported {
        "original"
    } else {
        "manual"
    };
    d.exchange_rate.map(|_| source.to_string())
}

/// An imported draft's number already used by another issued document.
fn duplicate_number(e: sea_orm::DbErr) -> AppError {
    number_violation(e, || AppError::field("number", "duplicate"))
}

pub fn apply_totals(row: &mut ActiveModel, t: &Totals) {
    row.total_base = Set(t.base);
    row.total_vat = Set(t.vat);
    row.total = Set(t.total);
    row.rounding = Set(t.rounding);
    row.payable = Set(t.payable);
    row.total_czk = Set(t.total_czk);
}

/// `data` and `totals` must already be validated / computed.
pub async fn create(
    db: &DatabaseConnection,
    space: SpaceId,
    data: DocumentData,
    totals: Totals,
) -> Result<Uuid, AppError> {
    Ok(db
        .transaction(|txn| Box::pin(create_in(txn, space, data, totals)))
        .await?)
}

pub async fn create_in(
    txn: &DatabaseTransaction,
    space: SpaceId,
    data: DocumentData,
    totals: Totals,
) -> Result<Uuid, AppError> {
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().into();
    advance_sources::lock_and_recheck(txn, space, id, &data.lines).await?;
    let d = &data;
    let row = NewDocument {
        id,
        space_id: space.uuid(),
        direction: Direction::Issued,
        doc_type: d.doc_type,
        status: Status::Draft,
        number: d.number.clone(),
        number_year: None,
        number_seq: None,
        imported: d.imported,
        contact_id: d.contact_id,
        issue_date: d.issue_date,
        tax_point_date: d.tax_point_date,
        due_date: Some(d.due_date),
        currency: d.currency.clone(),
        exchange_rate: d.exchange_rate,
        exchange_rate_date: None,
        exchange_rate_source: rate_source(d),
        locale: d.locale.clone(),
        vat_mode: d.vat_mode,
        bank_account_id: d.bank_account_id,
        payment_method: d.payment_method,
        variable_symbol: d.variable_symbol.clone(),
        constant_symbol: d.constant_symbol.clone(),
        order_ref: d.order_ref.clone(),
        header_note: d.header_note.clone(),
        footer_note: d.footer_note.clone(),
        internal_note: d.internal_note.clone(),
        rounding_mode: d.rounding_mode,
        supplier_snapshot: None,
        customer_snapshot: None,
        bank_snapshot: None,
        totals: totals.clone(),
        related_document_id: d.related_document_id,
        payment_id: None,
        correction_reason: d.correction_reason.clone(),
        supplier_number: None,
        received_date: None,
        vat_deductible: false,
        supplier_account: None,
        category_id: d.meta.category_id,
        custom_fields: serde_json::Value::Object(d.meta.custom_fields.clone()),
        original_path: None,
        original_sha256: None,
        original_size: None,
        original_uploaded_at: None,
        created_at: now,
        updated_at: now,
    };
    insert(txn, row).await.map_err(duplicate_number)?;
    lines::replace(txn, id, &data.lines).await?;
    lines::replace_recap(txn, id, &totals).await?;
    Ok(id)
}

/// Replace a draft; anything else is [`AppError::DocumentLocked`].
pub async fn update(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    data: DocumentData,
    totals: Totals,
) -> Result<(), AppError> {
    Ok(db
        .transaction(|txn| Box::pin(update_in(txn, space, id, data, totals)))
        .await?)
}

async fn update_in(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
    data: DocumentData,
    totals: Totals,
) -> Result<(), AppError> {
    let doc = locked_draft(txn, space, id).await?;
    advance_sources::lock_and_recheck(txn, space, id, &data.lines).await?;
    let unlocked = doc.unlocked_at.is_some();
    let mut row: ActiveModel = doc.into();
    if unlocked {
        row.edited_since_unlock = Set(true);
    }
    apply(&mut row, &data);
    apply_totals(&mut row, &totals);
    row.updated_at = Set(chrono::Utc::now().into());
    row.update(txn).await.map_err(duplicate_number)?;
    lines::replace(txn, id, &data.lines).await?;
    lines::replace_recap(txn, id, &totals).await?;
    Ok(())
}

/// Returns the uploaded original's path (imported drafts) for the caller to
/// remove once the delete committed.
pub async fn delete(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
) -> Result<Option<String>, AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked_draft(txn, space, id).await?;
                // Only an unlocked advance can be deducted. Every deducting line
                // counts, a cancelled invoice's too: the FK has no ON DELETE action
                // (an advance line needs its source).
                let deducted = document_line::Entity::find()
                    .filter(document_line::Column::AdvanceDocumentId.eq(id))
                    .filter(document_line::Column::DocumentId.ne(id))
                    .count(txn)
                    .await?;
                if deducted > 0 {
                    return Err(AppError::AdvanceInUse);
                }
                // An unlocked draft may still be what corrections / a final
                // invoice point at; a cancelled one may lose its link.
                let referenced = Entity::find()
                    .filter(Column::SpaceId.eq(space))
                    .filter(Column::RelatedDocumentId.eq(id))
                    .filter(Column::Status.ne(Status::Cancelled))
                    .count(txn)
                    .await?;
                if referenced > 0 {
                    return Err(AppError::RelatedInUse);
                }
                // An unlocked correction goes like a cancelled one: only within
                // the same guards (debit-note cap, DDPP not deducted).
                if doc.unlocked_at.is_some() {
                    match doc.doc_type {
                        DocType::DebitNote => credit::check_debit_cancel(txn, &doc).await?,
                        DocType::AdvanceCreditNote => {
                            ddpp_correction::check_linked(txn, &doc).await?
                        }
                        _ => {}
                    }
                }
                Entity::delete_by_id(id).exec(txn).await?;
                Ok(doc.original_path)
            })
        })
        .await?)
}

async fn locked_draft(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
) -> Result<crate::document::entity::document::Model, AppError> {
    let doc = query::lock(txn, space, id).await?;
    if doc.status != Status::Draft {
        return Err(AppError::DocumentLocked);
    }
    Ok(doc)
}
