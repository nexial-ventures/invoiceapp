//! The one place a `documents` row is created. Every column is listed, so a
//! new column cannot silently fall back to its database default in one of
//! the creating paths (draft, received, DDPP, import).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use sea_orm::entity::prelude::DateTimeWithTimeZone;
use sea_orm::{ActiveModelTrait, DatabaseTransaction, DbErr, Set};
use uuid::Uuid;

use crate::document::compute::Totals;
use crate::document::doc_type::DocType;
use crate::document::entity::document::ActiveModel;
use crate::document::line::{Direction, PaymentMethod, RoundingMode, Status, VatMode};

/// A new document row. Columns a new row never has yet (payments, sending,
/// cancellation, the rendered PDF archive) are not part of it.
#[derive(Debug, Clone)]
pub struct NewDocument {
    pub id: Uuid,
    pub space_id: Uuid,
    pub direction: Direction,
    pub doc_type: DocType,
    pub status: Status,
    pub number: Option<String>,
    pub number_year: Option<i32>,
    pub number_seq: Option<i32>,
    pub imported: bool,
    pub contact_id: Option<Uuid>,
    pub issue_date: NaiveDate,
    pub tax_point_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub currency: String,
    pub exchange_rate: Option<Decimal>,
    pub exchange_rate_date: Option<NaiveDate>,
    pub exchange_rate_source: Option<String>,
    pub locale: String,
    pub vat_mode: VatMode,
    pub bank_account_id: Option<Uuid>,
    pub payment_method: PaymentMethod,
    pub variable_symbol: Option<String>,
    pub constant_symbol: Option<String>,
    pub order_ref: Option<String>,
    pub header_note: Option<String>,
    pub footer_note: Option<String>,
    pub internal_note: Option<String>,
    pub rounding_mode: RoundingMode,
    pub supplier_snapshot: Option<serde_json::Value>,
    pub customer_snapshot: Option<serde_json::Value>,
    pub bank_snapshot: Option<serde_json::Value>,
    pub totals: Totals,
    pub related_document_id: Option<Uuid>,
    pub payment_id: Option<Uuid>,
    pub correction_reason: Option<String>,
    pub supplier_number: Option<String>,
    pub received_date: Option<NaiveDate>,
    pub vat_deductible: bool,
    pub supplier_account: Option<String>,
    pub category_id: Option<Uuid>,
    pub custom_fields: serde_json::Value,
    pub original_path: Option<String>,
    pub original_sha256: Option<String>,
    pub original_size: Option<i64>,
    pub original_uploaded_at: Option<DateTimeWithTimeZone>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

/// Insert `n`; the caller maps a number conflict to its own error.
pub async fn insert(txn: &DatabaseTransaction, n: NewDocument) -> Result<(), DbErr> {
    let t = n.totals;
    ActiveModel {
        id: Set(n.id),
        space_id: Set(n.space_id),
        direction: Set(n.direction),
        doc_type: Set(n.doc_type),
        status: Set(n.status),
        number: Set(n.number),
        number_year: Set(n.number_year),
        number_seq: Set(n.number_seq),
        imported: Set(n.imported),
        contact_id: Set(n.contact_id),
        issue_date: Set(n.issue_date),
        tax_point_date: Set(n.tax_point_date),
        due_date: Set(n.due_date),
        currency: Set(n.currency),
        exchange_rate: Set(n.exchange_rate),
        exchange_rate_date: Set(n.exchange_rate_date),
        exchange_rate_source: Set(n.exchange_rate_source),
        locale: Set(n.locale),
        vat_mode: Set(n.vat_mode),
        bank_account_id: Set(n.bank_account_id),
        payment_method: Set(n.payment_method),
        variable_symbol: Set(n.variable_symbol),
        constant_symbol: Set(n.constant_symbol),
        order_ref: Set(n.order_ref),
        header_note: Set(n.header_note),
        footer_note: Set(n.footer_note),
        internal_note: Set(n.internal_note),
        rounding_mode: Set(n.rounding_mode),
        supplier_snapshot: Set(n.supplier_snapshot),
        customer_snapshot: Set(n.customer_snapshot),
        bank_snapshot: Set(n.bank_snapshot),
        total_base: Set(t.base),
        total_vat: Set(t.vat),
        total: Set(t.total),
        rounding: Set(t.rounding),
        payable: Set(t.payable),
        total_czk: Set(t.total_czk),
        paid: Set(Decimal::ZERO),
        sent_at: Set(None),
        cancelled_at: Set(None),
        cancel_reason: Set(None),
        related_document_id: Set(n.related_document_id),
        payment_id: Set(n.payment_id),
        correction_reason: Set(n.correction_reason),
        pdf_path: Set(None),
        pdf_sha256: Set(None),
        pdf_rendered_at: Set(None),
        pdf_size: Set(None),
        supplier_number: Set(n.supplier_number),
        received_date: Set(n.received_date),
        vat_deductible: Set(n.vat_deductible),
        supplier_account: Set(n.supplier_account),
        category_id: Set(n.category_id),
        custom_fields: Set(n.custom_fields),
        original_path: Set(n.original_path),
        original_sha256: Set(n.original_sha256),
        original_size: Set(n.original_size),
        original_uploaded_at: Set(n.original_uploaded_at),
        unlocked_at: Set(None),
        unlocked_by: Set(None),
        unlock_reason: Set(None),
        created_at: Set(n.created_at),
        updated_at: Set(n.updated_at),
    }
    .insert(txn)
    .await?;
    Ok(())
}
