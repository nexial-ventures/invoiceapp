//! Response shapes and small request bodies of the document routes.

use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub use super::line_out::{Line, lines_out};
use crate::document::compute::{self, RecapRow};
use crate::document::custom_fields::Values;
use crate::document::line::{PaymentState, RoundingMode, Status};

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Recap {
    pub vat_rate: Decimal,
    pub base: Decimal,
    pub vat: Decimal,
    pub base_czk: Option<Decimal>,
    pub vat_czk: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    /// Sorted by rate, highest first.
    pub recap: Vec<Recap>,
    pub base: Decimal,
    pub vat: Decimal,
    pub total: Decimal,
    pub rounding: Decimal,
    pub payable: Decimal,
    pub total_czk: Option<Decimal>,
}

impl From<compute::Totals> for Totals {
    fn from(t: compute::Totals) -> Self {
        Self {
            recap: t
                .recap
                .into_iter()
                .map(|r: RecapRow| Recap {
                    vat_rate: r.vat_rate.normalize(),
                    base: r.base,
                    vat: r.vat,
                    base_czk: r.base_czk,
                    vat_czk: r.vat_czk,
                })
                .collect(),
            base: t.base,
            vat: t.vat,
            total: t.total,
            rounding: t.rounding,
            payable: t.payable,
            total_czk: t.total_czk,
        }
    }
}

/// Party data frozen at issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PartySnapshot {
    pub name: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub street: String,
    pub city: String,
    pub zip: String,
    pub country: String,
    pub registration: Option<String>,
    pub vat_payer: Option<bool>,
    /// Contact lines printed on the PDF (absent in pre-1d snapshots).
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub web: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BankSnapshot {
    pub account_number: Option<String>,
    pub iban: Option<String>,
    pub bic: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: Uuid,
    pub doc_type: String,
    pub direction: String,
    pub number: Option<String>,
    pub status: Status,
    pub payment_state: Option<PaymentState>,
    pub overdue: bool,
    pub contact_id: Option<Uuid>,
    pub issue_date: NaiveDate,
    pub tax_point_date: Option<NaiveDate>,
    /// `null` only on a received advance tax document.
    pub due_date: Option<NaiveDate>,
    pub currency: String,
    pub exchange_rate: Option<Decimal>,
    pub exchange_rate_date: Option<NaiveDate>,
    /// `cnb` | `manual`.
    pub exchange_rate_source: Option<String>,
    pub locale: String,
    pub vat_mode: String,
    pub bank_account_id: Option<Uuid>,
    pub payment_method: String,
    pub variable_symbol: Option<String>,
    pub constant_symbol: Option<String>,
    pub order_ref: Option<String>,
    pub header_note: Option<String>,
    pub footer_note: Option<String>,
    pub internal_note: Option<String>,
    /// `none` | `half` (nearest 0.50) | `unit` (whole units); derived from
    /// the amounts on received / imported documents.
    pub rounding_mode: RoundingMode,
    pub sent_at: Option<DateTime<FixedOffset>>,
    pub cancelled_at: Option<DateTime<FixedOffset>>,
    pub cancel_reason: Option<String>,
    pub supplier: Option<PartySnapshot>,
    pub customer: Option<PartySnapshot>,
    pub bank_snapshot: Option<BankSnapshot>,
    pub lines: Vec<Line>,
    pub totals: Totals,
    /// Sum of payments.
    pub paid: Decimal,
    /// credit_note → invoice, invoice → the proforma it settles, DDPP → proforma.
    pub related_document_id: Option<Uuid>,
    /// DDPP only: the proforma payment it documents.
    pub payment_id: Option<Uuid>,
    pub correction_reason: Option<String>,
    /// Documents whose `relatedDocumentId` is this one.
    pub related_documents: Vec<RelatedDocument>,
    /// The document `relatedDocumentId` points at.
    pub parent: Option<RelatedDocument>,
    /// Proforma only: a non-cancelled invoice settles it.
    pub settled: Option<bool>,
    /// Issued DDPP only: why `credit-note` is refused now — `advance_settled`
    /// (an issued invoice deducts it) > `advance_in_use` (a draft one does) >
    /// `fully_corrected` (nothing left); `null` otherwise.
    pub correction_block: Option<String>,
    /// `-1` for credit notes (amounts are stored positive), else `1`.
    pub sign: i8,
    /// The archived PDF (issued documents; a DDPP may lack it until its first download).
    pub pdf: Option<PdfArchive>,
    /// Manually imported issued document (keeps its own number, no rendered PDF).
    pub imported: bool,
    /// Received: the supplier's own document number.
    pub supplier_number: Option<String>,
    pub received_date: Option<NaiveDate>,
    pub vat_deductible: bool,
    pub supplier_account: Option<String>,
    pub category_id: Option<Uuid>,
    #[schema(value_type = Object)]
    pub custom_fields: Values,
    /// Uploaded original PDF (received / imported documents).
    pub original: Option<OriginalPdf>,
    /// Received: the recap as entered (`null` for issued documents).
    pub vat_recap: Option<Vec<EnteredRecap>>,
    /// `totals.rounding` / `totals.payable`; `total` = `totals.total`, except
    /// for received documents: Σ(base + vat) + rounding (as entered).
    pub rounding: Decimal,
    pub total: Decimal,
    pub payable: Decimal,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OriginalPdf {
    pub sha256: String,
    pub size: i64,
    pub uploaded_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnteredRecap {
    pub rate: Decimal,
    pub base: Decimal,
    pub vat: Decimal,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PdfArchive {
    pub sha256: String,
    pub rendered_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelatedDocument {
    pub id: Uuid,
    pub doc_type: String,
    pub number: Option<String>,
    pub status: Status,
    pub payable: Decimal,
    /// The related document's own currency.
    pub currency: String,
}

/// `-1` for credit notes and DDPP corrections (stored positive).
pub fn sign(doc_type: &str) -> i8 {
    crate::settings::doc_type::DocType::parse_document(doc_type).map_or(1, |t| t.sign())
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub id: Uuid,
    pub doc_type: String,
    pub direction: String,
    pub number: Option<String>,
    pub status: Status,
    pub payment_state: Option<PaymentState>,
    pub overdue: bool,
    pub contact_id: Option<Uuid>,
    /// The counterparty (received: the supplier): snapshot name once
    /// issued / recorded, the live contact name for drafts.
    pub customer_name: Option<String>,
    pub issue_date: NaiveDate,
    pub due_date: Option<NaiveDate>,
    pub currency: String,
    pub payable: Decimal,
    pub paid: Decimal,
    pub sent_at: Option<DateTime<FixedOffset>>,
    pub sign: i8,
    pub related_document_id: Option<Uuid>,
    pub imported: bool,
    pub supplier_number: Option<String>,
    pub category_id: Option<Uuid>,
    /// A rendered archive or an uploaded original exists.
    pub has_pdf: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DocumentList {
    pub items: Vec<DocumentSummary>,
    pub total: u64,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    pub direction: Option<String>,
    pub doc_type: Option<String>,
    pub status: Option<Status>,
    pub payment_state: Option<PaymentState>,
    /// `true`: only overdue; `false`: only not overdue.
    pub overdue: Option<bool>,
    pub contact_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub imported: Option<bool>,
    /// Case-insensitive substring of number, counterparty name, variable
    /// symbol or supplier number.
    pub q: Option<String>,
    /// Inclusive `issueDate` range.
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    /// Default 50, clamped to 1..=200.
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Computed {
    pub lines: Vec<Line>,
    pub totals: Totals,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CancelInput {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct MarkSentInput {
    /// Default now.
    pub sent_at: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct InternalNoteInput {
    pub internal_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Payment {
    pub id: Uuid,
    pub date: NaiveDate,
    pub amount: Decimal,
    pub note: Option<String>,
    /// The DDPP this payment created (payer proformas).
    pub advance_document_id: Option<Uuid>,
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct PaymentInput {
    pub date: Option<NaiveDate>,
    /// Decimal string > 0, at most 2 dp, document currency.
    #[serde(deserialize_with = "crate::num_text::decimal")]
    pub amount: String,
    pub note: Option<String>,
    /// Foreign-currency proforma of a VAT payer: CZK per unit for the DDPP
    /// (default: ČNB for the payment date). Ignored otherwise.
    #[serde(deserialize_with = "crate::num_text::opt_decimal")]
    pub exchange_rate: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CreditNoteInput {
    /// At most 500 characters; required at issue.
    pub correction_reason: Option<String>,
}
