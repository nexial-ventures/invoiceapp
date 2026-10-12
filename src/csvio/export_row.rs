//! One stored document → its export row ([`OutRow`]). Pure.

use anyhow::Context as _;
use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};

use super::write::OutRow;
use crate::document::entity::{document, payment, vat_recap};
use crate::document::line::{PaymentState, Status, VatMode};
use crate::document::snapshot::PartySnapshot;
use crate::document::state::document_payment_state;
use crate::import::model::Party;
use crate::settings::doc_type::{DocType, ISSUED, RECEIVED};

/// A document and what its row needs from other tables.
pub struct Source<'a> {
    pub doc: &'a document::Model,
    pub recap: &'a [vat_recap::Model],
    /// Its payments, any order.
    pub payments: &'a [payment::Model],
    /// The document `related_document_id` points at.
    pub parent: Option<&'a document::Model>,
    pub category: Option<&'a str>,
}

/// The date of the payment that made the document fully paid (`paid` /
/// `overpaid`); `None` otherwise, also when it is settled without any
/// payment (a payable of 0).
pub fn paid_date(
    doc_type: &str,
    status: Status,
    paid: Decimal,
    payable: Decimal,
    payments: &[payment::Model],
) -> Option<NaiveDate> {
    let state = document_payment_state(doc_type, status, paid, payable)?;
    if !matches!(state, PaymentState::Paid | PaymentState::Overpaid) {
        return None;
    }
    let mut ordered: Vec<&payment::Model> = payments.iter().collect();
    ordered.sort_by_key(|p| (p.date, p.created_at, p.id));
    let mut sum = Decimal::ZERO;
    ordered.into_iter().find_map(|p| {
        sum += p.amount;
        (sum >= payable).then_some(p.date)
    })
}

/// A stored counterparty snapshot, decoded (`None` when there is none).
pub fn snapshot(snapshot: &Option<serde_json::Value>) -> anyhow::Result<Option<PartySnapshot>> {
    snapshot
        .as_ref()
        .map(|v| serde_json::from_value(v.clone()).context("decode counterparty snapshot"))
        .transpose()
}

/// The counterparty of a stored snapshot (`None` when there is none).
pub fn party(stored: &Option<serde_json::Value>) -> anyhow::Result<Option<Party>> {
    let Some(s) = snapshot(stored)? else {
        return Ok(None);
    };
    Ok(Some(Party {
        name: s.name,
        ico: s.ico,
        dic: s.dic,
        street: s.street,
        city: s.city,
        zip: s.zip,
        country: s.country,
        registration: s.registration,
        email: s.email,
        phone: s.phone,
        web: s.web,
    }))
}

/// A CZK amount of a foreign recap row; computed from the rate only when a
/// (legacy) row lacks it, `None` when there is no rate either.
fn czk(stored: Option<Decimal>, amount: Decimal, rate: Option<Decimal>) -> Option<Decimal> {
    stored.or_else(|| {
        rate.map(|r| (amount * r).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero))
    })
}

/// The stored direction, type and VAT mode of a document, decoded; an
/// unknown stored value is an error (the export skips or refuses it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    pub direction: &'static str,
    pub issued: bool,
    pub doc_type: DocType,
    pub vat_mode: VatMode,
}

impl Kind {
    pub fn of(doc: &document::Model) -> anyhow::Result<Self> {
        let (direction, issued) = match doc.direction.as_str() {
            ISSUED => (ISSUED, true),
            RECEIVED => (RECEIVED, false),
            other => anyhow::bail!("stored direction {other}"),
        };
        Ok(Self {
            direction,
            issued,
            doc_type: DocType::parse_document(&doc.doc_type).context("stored document type")?,
            vat_mode: VatMode::parse(&doc.vat_mode).context("stored VAT mode")?,
        })
    }
}

/// The stored recap in CZK as `(rate, base, VAT)`, positive as stored: a
/// CZK document's as is, a foreign one's `baseCzk` / `vatCzk`.
pub fn czk_recap(
    doc: &document::Model,
    recap: &[vat_recap::Model],
) -> anyhow::Result<Vec<(Decimal, Decimal, Decimal)>> {
    let is_czk = doc.currency == "CZK";
    let rate = (!is_czk).then_some(doc.exchange_rate).flatten();
    recap
        .iter()
        .map(|r| match is_czk {
            true => Some((r.vat_rate, r.base, r.vat)),
            false => Some((
                r.vat_rate,
                czk(r.base_czk, r.base, rate)?,
                czk(r.vat_czk, r.vat, rate)?,
            )),
        })
        .collect::<Option<Vec<_>>>()
        .context("foreign recap without a CZK amount or an exchange rate")
}

pub fn out_row(s: &Source) -> anyhow::Result<OutRow> {
    let doc = s.doc;
    let Kind {
        direction,
        issued,
        doc_type,
        vat_mode,
    } = Kind::of(doc)?;
    let status = Status::parse(&doc.status).context("stored status")?;
    let is_czk = doc.currency == "CZK";
    let rate = (!is_czk).then_some(doc.exchange_rate).flatten();
    let recap = czk_recap(doc, s.recap)?;
    let total = doc.total + doc.rounding;
    Ok(OutRow {
        direction,
        doc_type,
        number: doc.number.clone(),
        supplier_number: doc.supplier_number.clone().filter(|_| !issued),
        related_number: s.parent.and_then(|p| match issued {
            true => p.number.clone(),
            false => p.supplier_number.clone(),
        }),
        issue_date: doc.issue_date,
        tax_date: doc.tax_point_date,
        due_date: doc.due_date,
        received_date: doc.received_date.filter(|_| !issued),
        counterparty: party(match issued {
            true => &doc.customer_snapshot,
            false => &doc.supplier_snapshot,
        })?,
        currency: doc.currency.clone(),
        exchange_rate: rate,
        vat_mode,
        recap,
        rounding: doc.rounding,
        total,
        total_czk: if is_czk { Some(total) } else { doc.total_czk },
        paid_date: paid_date(&doc.doc_type, status, doc.paid, doc.payable, s.payments),
        variable_symbol: doc.variable_symbol.clone(),
        vat_deductible: (!issued).then_some(doc.vat_deductible),
        category: s.category.map(str::to_string),
        note: match issued {
            true => doc.header_note.clone(),
            false => doc.internal_note.clone(),
        },
    })
}

#[cfg(test)]
#[path = "export_row_tests.rs"]
mod tests;
