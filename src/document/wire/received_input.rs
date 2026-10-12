//! Received document request body (`direction: "received"`) and its
//! validation into [`ReceivedData`].

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::fields::{digits, exchange_rate, parse_vat_mode};
use crate::document::custom_fields::Values;
use crate::document::line::{MAX_AMOUNT, VatMode};
use crate::document::meta;
use crate::document::model::{ReceivedCtx, ReceivedData};
use crate::document::received::{self, EnteredRow};
use crate::error::{AppError, FieldErrors};
use crate::num_parse::{decimal, parse_rate};
use crate::settings::doc_type::{DocType, RECEIVED};
use crate::validation::{self as v, Check};

pub const MAX_RECAP_ROWS: usize = 50;

/// Create / replace body of a received document. `POST` defaults
/// `docType` (`invoice`), `currency` (`CZK`) and `vatMode` (`standard`);
/// `PUT` requires `currency` and `vatMode`.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ReceivedInput {
    /// Any of the seven document types (default `invoice`); fixed after create.
    pub doc_type: Option<String>,
    /// `received`.
    pub direction: Option<String>,
    pub supplier_number: Option<String>,
    pub contact_id: Option<Uuid>,
    pub issue_date: Option<NaiveDate>,
    pub tax_point_date: Option<NaiveDate>,
    /// Default `taxPointDate ?? issueDate`.
    pub received_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub currency: Option<String>,
    /// Manual override; default: ČNB for `receivedDate`.
    pub exchange_rate: Option<String>,
    pub vat_mode: Option<String>,
    pub vat_recap: Vec<RecapInput>,
    /// Default `"0"`, `|rounding| < 100`.
    pub rounding: Option<String>,
    pub payable: Option<String>,
    /// Default: `vatMode == standard`.
    pub vat_deductible: Option<bool>,
    pub variable_symbol: Option<String>,
    pub constant_symbol: Option<String>,
    pub supplier_account: Option<String>,
    pub related_document_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    #[schema(value_type = Object)]
    pub custom_fields: Option<Values>,
    pub internal_note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct RecapInput {
    pub rate: Option<String>,
    pub base: Option<String>,
    pub vat: Option<String>,
}

fn doc_type(requested: Option<&str>, existing: Option<DocType>) -> Check<DocType> {
    let parsed = match requested.map(str::trim) {
        Some(s) => Some(DocType::parse_document(s).ok_or("invalid")?),
        None => None,
    };
    match (existing, parsed) {
        (Some(x), Some(t)) if t != x => Err("invalid"),
        (_, Some(t)) => Ok(t),
        (Some(x), None) => Ok(x),
        (None, None) => Ok(DocType::Invoice),
    }
}

/// Recap rows: >= 1, rate unique (0..100, 2 dp — any rate, not only the
/// configured ones), base / vat 2 dp; vat must be 0 unless `standard`.
fn recap(rows: &[RecapInput], mode: Option<VatMode>, e: &mut FieldErrors) -> Vec<EnteredRow> {
    if rows.is_empty() {
        e.add("vatRecap", "required");
    } else if rows.len() > MAX_RECAP_ROWS {
        e.add("vatRecap", "too_long");
        return Vec::new();
    }
    let mut out: Vec<EnteredRow> = Vec::with_capacity(rows.len());
    let mut seen: Vec<Decimal> = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        let f = |name: &str| format!("vatRecap.{i}.{name}");
        let amount = |s: &Option<String>| decimal(s.as_deref().unwrap_or_default(), 2, MAX_AMOUNT);
        let rate = e.check(
            &f("rate"),
            parse_rate(r.rate.as_deref().unwrap_or_default()),
        );
        let rate = rate.filter(|rate| {
            let dup = seen.contains(rate);
            seen.push(*rate);
            if dup {
                e.add(&f("rate"), "duplicate");
            }
            !dup
        });
        let base = e.check(&f("base"), amount(&r.base));
        let vat = e.check(&f("vat"), amount(&r.vat)).filter(|vat| {
            let charged = mode.is_none_or(VatMode::charges_vat) || vat.is_zero();
            if !charged {
                e.add(&f("vat"), "invalid");
            }
            charged
        });
        if let (Some(rate), Some(base), Some(vat)) = (rate, base, vat) {
            out.push(EnteredRow { rate, base, vat });
        }
    }
    out
}

impl ReceivedInput {
    /// Normalize and validate every field (all failures at once).
    pub fn validate(self, ctx: &ReceivedCtx) -> Result<ReceivedData, AppError> {
        let mut e = FieldErrors::new();
        let defaults = ctx.existing.is_none();
        let existing_type = ctx.existing.map(|x| x.1);
        let doc_type = e
            .check("docType", doc_type(self.doc_type.as_deref(), existing_type))
            .unwrap_or(DocType::Invoice);
        let supplier_number = e.check(
            "supplierNumber",
            v::required_text(self.supplier_number.as_deref().unwrap_or_default(), 40),
        );
        let contact_id = match (self.contact_id, &ctx.contact) {
            (None, _) => e.check("contactId", Err("required")),
            (Some(_), None) => e.check("contactId", Err("invalid")),
            (Some(id), Some(_)) => Some(id),
        };
        let issue_date = e.check("issueDate", self.issue_date.ok_or("required"));
        let tax_point_date = match (doc_type, self.tax_point_date) {
            (DocType::Proforma, Some(_)) => e.check("taxPointDate", Err("invalid")),
            (DocType::Proforma, None) => None,
            (_, t) => e.check("taxPointDate", t.ok_or("required")),
        };
        let received_date = self.received_date.or(tax_point_date).or(issue_date);
        let due_date = match (doc_type, self.due_date) {
            // Only a received DDPP may have no due date (its correction is a
            // refund like a credit note: due date required).
            (DocType::AdvanceTaxDoc, d) => d,
            (_, d) => e.check("dueDate", d.ok_or("required")),
        };
        let currency = match self.currency.as_deref() {
            Some(s) => e.check("currency", v::currency(s)),
            None if defaults => Some("CZK".to_string()),
            None => e.check("currency", Err("required")),
        }
        .unwrap_or_else(|| "CZK".into());
        let rate = e
            .check("exchangeRate", exchange_rate(self.exchange_rate.as_deref()))
            .flatten()
            .filter(|_| currency != "CZK");
        let vat_mode = match self.vat_mode.as_deref() {
            Some(s) => e.check("vatMode", parse_vat_mode(s)),
            None if defaults => Some(VatMode::Standard),
            None => e.check("vatMode", Err("required")),
        };
        let rows = recap(&self.vat_recap, vat_mode, &mut e);
        let rounding = match self.rounding.as_deref().map(str::trim) {
            None | Some("") => Some(Decimal::ZERO),
            Some(s) => e.check("rounding", decimal(s, 2, Decimal::ONE_HUNDRED)),
        };
        let payable = e.check(
            "payable",
            decimal(self.payable.as_deref().unwrap_or_default(), 2, MAX_AMOUNT).and_then(|p| {
                if p < Decimal::ZERO {
                    Err("invalid")
                } else {
                    Ok(p)
                }
            }),
        );
        let self_id = ctx.existing.map(|x| x.0);
        let related_document_id = meta::check_related(
            self.related_document_id,
            doc_type,
            RECEIVED,
            self_id,
            ctx.related.as_ref(),
            &mut e,
        );
        let meta = meta::validate(
            self.category_id,
            self.custom_fields.as_ref(),
            RECEIVED,
            &ctx.meta,
            &mut e,
        );
        let data = ReceivedData {
            doc_type,
            supplier_number: supplier_number.unwrap_or_default(),
            contact_id: contact_id.unwrap_or_default(),
            issue_date: issue_date.unwrap_or_default(),
            tax_point_date,
            received_date: received_date.unwrap_or_default(),
            due_date,
            currency,
            exchange_rate: rate,
            vat_mode: vat_mode.unwrap_or(VatMode::Standard),
            vat_deductible: self
                .vat_deductible
                .unwrap_or(vat_mode == Some(VatMode::Standard)),
            recap: rows,
            rounding: rounding.unwrap_or_default(),
            payable: payable.unwrap_or_default(),
            variable_symbol: e
                .check(
                    "variableSymbol",
                    digits(self.variable_symbol.as_deref(), 10),
                )
                .flatten(),
            constant_symbol: e
                .check("constantSymbol", digits(self.constant_symbol.as_deref(), 4))
                .flatten(),
            supplier_account: e
                .check(
                    "supplierAccount",
                    v::opt_text(self.supplier_account.as_deref(), 60),
                )
                .flatten(),
            related_document_id,
            meta,
            internal_note: e
                .check(
                    "internalNote",
                    v::opt_text(self.internal_note.as_deref(), 2000),
                )
                .flatten(),
        };
        e.into_result()?;
        // Sums that would not fit the money columns.
        received::totals(&data.recap, data.rounding, data.payable, None)
            .map_err(|_| AppError::field("vatRecap", "invalid"))?;
        Ok(data)
    }
}

#[cfg(test)]
#[path = "received_input_tests.rs"]
mod tests;
