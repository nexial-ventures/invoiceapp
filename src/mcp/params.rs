//! Tool inputs that are not a REST body as is (those — `DocumentInput`,
//! `ContactInput`, `ComputeInput` — are used directly).

use chrono::{DateTime, FixedOffset, NaiveDate};
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use crate::document::line::{PaymentState, Status};
use crate::document::wire::input::DocumentInput;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListDocuments {
    /// `issued` (our invoices) or `received` (supplier invoices).
    pub direction: String,
    /// e.g. `invoice`, `proforma`, `simplified`, `credit_note`, `advance_tax_doc`.
    pub doc_type: Option<String>,
    pub status: Option<Status>,
    pub payment_state: Option<PaymentState>,
    /// `true`: only overdue; `false`: only not overdue.
    pub overdue: Option<bool>,
    pub contact_id: Option<Uuid>,
    /// Case-insensitive substring of number, counterparty name, variable symbol or supplier number.
    pub q: Option<String>,
    /// Inclusive `issueDate` range start (YYYY-MM-DD).
    pub from: Option<NaiveDate>,
    /// Inclusive `issueDate` range end (YYYY-MM-DD).
    pub to: Option<NaiveDate>,
    /// Page size, default 20, at most 100.
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Id {
    pub id: Uuid,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListContacts {
    /// Case-insensitive substring of name, IČO, DIČ or city.
    pub q: Option<String>,
    /// Page size, default 20, at most 100.
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Ares {
    /// 8-digit Czech company id (IČO).
    pub ico: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    /// Case-insensitive substring of the item name.
    pub q: Option<String>,
}

/// Schema only: `update_draft` parses `id` and the document apart, so field
/// paths inside the flattened document survive.
#[derive(Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(dead_code, reason = "input schema of update_draft only")]
pub struct UpdateDraft {
    /// The draft to replace.
    pub id: Uuid,
    /// Every field is replaced (omitted = cleared / required error), as `PUT /api/documents/{id}`.
    #[serde(flatten)]
    pub document: DocumentInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddPayment {
    /// The issued document.
    pub id: Uuid,
    /// Payment date (YYYY-MM-DD).
    pub date: NaiveDate,
    /// Decimal string > 0, at most 2 dp, in the document currency.
    #[serde(deserialize_with = "crate::num_text::decimal")]
    #[schemars(with = "crate::num_text::DecimalText")]
    pub amount: String,
    pub note: Option<String>,
    /// Foreign-currency proforma of a VAT payer: CZK per unit for its DDPP
    /// (default: ČNB for the payment date). Ignored otherwise.
    #[serde(default, deserialize_with = "crate::num_text::opt_decimal")]
    #[schemars(with = "Option<crate::num_text::DecimalText>")]
    pub exchange_rate: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkSent {
    /// The issued document.
    pub id: Uuid,
    /// RFC 3339 timestamp; default now.
    pub sent_at: Option<DateTime<FixedOffset>>,
}
