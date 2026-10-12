//! Validated document data and what validating it needs from the database:
//! the domain side of the issued and received request bodies (`wire`), used
//! by the repos and the document service.

use std::collections::HashMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::contact::entity::contact;
use crate::document::advance::{self, AdvanceCtx, AdvanceSource};
use crate::document::compute::{self, Evaluated, Params};
use crate::document::correction::ExactBasis;
use crate::document::entity::document;
use crate::document::line::{LineData, PaymentMethod, RoundingMode, VatMode};
use crate::document::meta::{Meta, MetaCtx};
use crate::document::received::EnteredRow;
use crate::error::{AppError, FieldErrors};
use crate::settings::doc_type::DocType;
use crate::settings::entity::company;

/// The draft being updated, as far as validation cares.
#[derive(Debug, Clone)]
pub struct Existing {
    pub id: Uuid,
    pub doc_type: DocType,
    pub imported: bool,
    pub related_document_id: Option<Uuid>,
    pub contact_id: Option<Uuid>,
    pub currency: String,
    pub locale: String,
    pub vat_mode: VatMode,
    /// A correction keeps its original's rate whatever the request says.
    pub exchange_rate: Option<Decimal>,
}

impl Existing {
    /// A correction stays bound to its original's contact, currency and VAT
    /// mode (copied at creation): anything else is `invalid`.
    pub fn check_bound(
        &self,
        currency: &str,
        contact_id: Option<Uuid>,
        vat_mode: VatMode,
        e: &mut FieldErrors,
    ) {
        if currency != self.currency {
            e.add("currency", "invalid");
        }
        if contact_id != self.contact_id {
            e.add("contactId", "invalid");
        }
        if vat_mode != self.vat_mode {
            e.add("vatMode", "invalid");
        }
    }
}

/// Validated [`DocumentInput`].
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentData {
    pub doc_type: DocType,
    /// Set by settle / credit-note creation; kept on update.
    pub related_document_id: Option<Uuid>,
    pub correction_reason: Option<String>,
    pub contact_id: Option<Uuid>,
    pub issue_date: NaiveDate,
    pub tax_point_date: Option<NaiveDate>,
    pub due_date: NaiveDate,
    pub currency: String,
    pub exchange_rate: Option<Decimal>,
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
    pub lines: Vec<LineData>,
    pub imported: bool,
    /// Imported documents: their own number.
    pub number: Option<String>,
    pub meta: Meta,
}

impl DocumentData {
    pub fn params(&self) -> Params {
        Params {
            vat_mode: self.vat_mode,
            is_czk: self.currency == "CZK",
            exchange_rate: self.exchange_rate,
            rounding: self.rounding_mode,
        }
    }
}

/// What validation needs from the database.
pub struct Context {
    pub today: NaiveDate,
    /// `POST`: fill omitted fields from contact/company; `PUT`: they are required.
    pub apply_defaults: bool,
    pub company: company::Model,
    /// The contact named by `contactId`, if it exists.
    pub contact: Option<contact::Model>,
    pub default_vat_rate: Option<Decimal>,
    /// `PUT`: the draft being replaced.
    pub existing: Option<Existing>,
    /// Documents referenced by `advance` lines, by id.
    pub advances: HashMap<Uuid, AdvanceSource>,
    /// The document named by `relatedDocumentId`, if it exists.
    pub related: Option<document::Model>,
    pub meta: MetaCtx,
    /// `PUT` of a native DDPP correction: the basis of its exact VAT.
    pub exact: Option<ExactBasis>,
}

/// Resolve advance lines, then compute; errors of both are reported together.
/// `exact`: a native DDPP correction's exact-VAT basis (save / compute; issue
/// passes `None` and re-applies it under the DDPP lock).
pub fn evaluate(
    lines: &mut [LineData],
    params: Params,
    adv: &AdvanceCtx,
    exact: Option<&ExactBasis>,
) -> Result<Evaluated, AppError> {
    let mut e = advance::resolve(lines, adv);
    match compute::evaluate(lines, params) {
        Ok(mut ev) if e.is_empty() => {
            if let Some(b) = exact {
                b.apply(&mut ev.totals, params)
                    .map_err(|o| AppError::field(o.field(), "invalid"))?;
            }
            Ok(ev)
        }
        Ok(_) => Err(AppError::Validation(e)),
        Err(more) => {
            e.merge(more);
            Err(AppError::Validation(e))
        }
    }
}

/// Validated [`ReceivedInput`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedData {
    pub doc_type: DocType,
    pub supplier_number: String,
    pub contact_id: Uuid,
    pub issue_date: NaiveDate,
    pub tax_point_date: Option<NaiveDate>,
    pub received_date: NaiveDate,
    pub due_date: Option<NaiveDate>,
    pub currency: String,
    /// The manual rate, if any.
    pub exchange_rate: Option<Decimal>,
    pub vat_mode: VatMode,
    pub recap: Vec<EnteredRow>,
    pub rounding: Decimal,
    pub payable: Decimal,
    pub vat_deductible: bool,
    pub variable_symbol: Option<String>,
    pub constant_symbol: Option<String>,
    pub supplier_account: Option<String>,
    pub related_document_id: Option<Uuid>,
    pub meta: Meta,
    pub internal_note: Option<String>,
}

/// What validation needs from the database.
#[derive(Debug, Clone, Default)]
pub struct ReceivedCtx {
    /// `PUT`: the stored document's id and type.
    pub existing: Option<(Uuid, DocType)>,
    /// The contact named by `contactId`, if it exists.
    pub contact: Option<contact::Model>,
    /// The document named by `relatedDocumentId`, if it exists.
    pub related: Option<document::Model>,
    pub meta: MetaCtx,
}
