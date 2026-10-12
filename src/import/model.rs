//! A document as an import source plans it: everything the shared lookups
//! and the per-entry transaction need, independent of the file format.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::currency::is_local;
use crate::document::compute::Totals;
use crate::document::doc_type::DocType;
use crate::document::line::Direction;
use crate::document::line::{LineData, PaymentMethod, VatMode};
use crate::document::snapshot::{BankSnapshot, PartySnapshot};

/// An entry error or warning code of the preview / confirm.
pub type Code = &'static str;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Party {
    pub name: String,
    /// Whitespace stripped.
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub street: String,
    pub city: String,
    pub zip: String,
    /// ISO 3166-1 alpha-2, default `CZ`.
    pub country: String,
    pub registration: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub web: Option<String>,
}

impl Party {
    /// The document snapshot of the party, as written.
    pub fn snapshot(&self, vat_payer: Option<bool>) -> PartySnapshot {
        PartySnapshot {
            name: self.name.clone(),
            ico: self.ico.clone(),
            dic: self.dic.clone(),
            street: self.street.clone(),
            city: self.city.clone(),
            zip: self.zip.clone(),
            country: self.country.clone(),
            registration: self.registration.clone(),
            vat_payer,
            email: self.email.clone(),
            phone: self.phone.clone(),
            web: self.web.clone(),
        }
    }
}

/// How the counterparty is matched to an existing contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactRule {
    /// ISDOC: by IČO; a party without one matches a contact without one of
    /// exactly the same name.
    IcoOrName,
    /// CSV: by IČO, else DIČ, else name (case-insensitive, trimmed); never a
    /// contact whose IČO or DIČ contradicts the party's.
    IcoDicName,
}

/// Where an issued document's bank account comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuedBank {
    /// The company account named by [`Plan::bank`]; the snapshot is
    /// `Plan::bank` as written.
    Payment,
    /// The default company account of the document currency.
    CurrencyDefault,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub direction: Direction,
    pub doc_type: DocType,
    /// Our number (issued) or the supplier's (received).
    pub number: String,
    pub supplier: Party,
    pub customer: Option<Party>,
    /// The supplier snapshot's `vatPayer`.
    pub vat_payer: Option<bool>,
    pub issue_date: NaiveDate,
    pub tax_point_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    /// Received only; `None` → the tax point, else the issue date.
    pub received_date: Option<NaiveDate>,
    pub currency: String,
    /// From the source; `None` for a foreign currency → ČNB.
    pub rate: Option<Decimal>,
    pub vat_mode: VatMode,
    pub lines: Vec<LineData>,
    pub totals: Totals,
    /// The total including VAT shown in the preview (document currency).
    pub gross: Decimal,
    pub payment_method: PaymentMethod,
    pub variable_symbol: Option<String>,
    pub constant_symbol: Option<String>,
    pub bank: Option<BankSnapshot>,
    pub note: Option<String>,
    pub locale: String,
    /// The number of the document this one corrects / settles.
    pub original_ref: Option<String>,
    pub warnings: Vec<Code>,
    pub contact_rule: ContactRule,
    pub issued_bank: IssuedBank,
}

impl Plan {
    /// The other party: the customer of an issued document, the supplier of
    /// a received one.
    pub fn counterparty(&self) -> Option<&Party> {
        if self.direction == Direction::Issued {
            self.customer.as_ref()
        } else {
            Some(&self.supplier)
        }
    }

    /// ČNB rate date: the tax point (received: the received date, which is
    /// the tax point too), else the issue date.
    pub fn rate_date(&self) -> NaiveDate {
        self.tax_point_date.unwrap_or(self.issue_date)
    }

    pub fn needs_cnb(&self) -> bool {
        !is_local(&self.currency) && self.rate.is_none()
    }

    pub fn received_date(&self) -> NaiveDate {
        self.received_date
            .or(self.tax_point_date)
            .unwrap_or(self.issue_date)
    }
}
