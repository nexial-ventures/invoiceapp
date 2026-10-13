//! The `GET /api/overview` response ([api/overview.md](../../docs/api/overview.md)).
//! Every amount is in CZK, signed (credit notes reduce), as a decimal string.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::document::doc_type::DocType;

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// The day receivables / payables are counted at (Europe/Prague).
    pub today: NaiveDate,
    /// The company is a VAT payer: `amount` is the base (excl. VAT), else the
    /// total; `false` → the client hides `vat`.
    pub vat_payer: bool,
    /// Issued documents of the period.
    pub income: Totals,
    /// Received documents of the period.
    pub expense: Totals,
    /// `income.amount − expense.amount`.
    pub difference: Decimal,
    pub vat: Vat,
    /// Issued documents owed to us, as of `today`.
    pub receivables: Open,
    /// Received documents we owe, as of `today`.
    pub payables: Open,
    pub months: Vec<Month>,
    pub categories: Categories,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewTotals)]
pub struct Totals {
    /// Documents counted.
    pub count: u32,
    /// By the amount basis (`vatPayer`): base, plus the non-deductible VAT of
    /// received documents; non-payer: total.
    pub amount: Decimal,
    pub base: Decimal,
    pub vat: Decimal,
    pub total: Decimal,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewVat)]
pub struct Vat {
    /// Issued `standard` documents per rate, highest rate first.
    pub output: Vec<VatRow>,
    /// Received `standard` documents with deductible VAT per rate.
    pub input: Vec<VatRow>,
    /// Σ output VAT.
    pub output_vat: Decimal,
    /// Σ deductible input VAT.
    pub input_vat: Decimal,
    /// `outputVat − inputVat`: positive = to pay, negative = refund.
    pub result: Decimal,
    /// Bases of issued reverse-charge documents (no VAT computed).
    pub reverse_charge_issued: Decimal,
    /// Bases of received reverse-charge documents (no VAT computed).
    pub reverse_charge_received: Decimal,
    /// Received `standard` documents without a VAT deduction (informative;
    /// their total counts as expense).
    pub non_deductible: VatRow,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewVatRow)]
pub struct VatRow {
    /// Percent; `0` in `nonDeductible` (it spans rates).
    pub rate: Decimal,
    pub base: Decimal,
    pub vat: Decimal,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewOpen)]
pub struct Open {
    /// Unpaid or partially paid tax documents (no proformas, no DDPPs).
    pub count: u32,
    /// Σ `payable − paid`.
    pub amount: Decimal,
    /// Of those, due before `today`.
    pub overdue_count: u32,
    pub overdue_amount: Decimal,
    /// The longest overdue (earliest due date first), at most 5.
    pub top_overdue: Vec<OpenDoc>,
    /// Unpaid, unsettled proformas: expected advances.
    pub advances: Advances,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewAdvances)]
pub struct Advances {
    pub count: u32,
    pub amount: Decimal,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewOpenDoc)]
pub struct OpenDoc {
    pub id: Uuid,
    pub doc_type: DocType,
    /// Issued: our number; received: the supplier's, else ours.
    pub number: Option<String>,
    pub counterparty: Option<String>,
    pub due_date: NaiveDate,
    /// `payable − paid` in CZK.
    pub remaining: Decimal,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewMonth)]
pub struct Month {
    /// `YYYY-MM`.
    pub month: String,
    pub income: Decimal,
    pub expense: Decimal,
    /// The month overlaps the selected period.
    pub in_period: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewCategories)]
pub struct Categories {
    pub income: Vec<CategoryAmount>,
    pub expense: Vec<CategoryAmount>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = OverviewCategoryAmount)]
pub struct CategoryAmount {
    /// `null` = uncategorized.
    pub id: Option<Uuid>,
    pub name: Option<String>,
    pub count: u32,
    pub amount: Decimal,
}
