//! A stored document as every export reads it ([`Source`]) and what the CSV
//! and the accounting program files decode from it the same way. Pure.

use anyhow::Context as _;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::currency::is_local;
use crate::document::doc_type::DocType;
use crate::document::entity::{document, payment, vat_recap};
use crate::document::line::{Direction, VatMode};
use crate::document::snapshot::PartySnapshot;
use crate::import::model::Party;

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

/// The direction, type and VAT mode of a stored document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    pub direction: Direction,
    pub issued: bool,
    pub doc_type: DocType,
    pub vat_mode: VatMode,
}

impl Kind {
    pub fn of(doc: &document::Model) -> Self {
        Self {
            direction: doc.direction,
            issued: doc.direction == Direction::Issued,
            doc_type: doc.doc_type,
            vat_mode: doc.vat_mode,
        }
    }
}

/// The stored recap in CZK as `(rate, base, VAT)`, positive as stored: a
/// CZK document's as is, a foreign one's `baseCzk` / `vatCzk`.
pub fn czk_recap(
    doc: &document::Model,
    recap: &[vat_recap::Model],
) -> anyhow::Result<Vec<(Decimal, Decimal, Decimal)>> {
    let is_czk = is_local(&doc.currency);
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
