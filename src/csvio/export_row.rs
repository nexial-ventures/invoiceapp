//! One stored document → its export row ([`OutRow`]). Pure.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::write::OutRow;
use crate::currency::is_local;
use crate::document::doc_type::DocType;
use crate::document::entity::payment;
use crate::document::line::{PaymentState, Status};
use crate::document::state::document_payment_state;
use crate::export::source::{Kind, Source, czk_recap, party};

/// The date of the payment that made the document fully paid (`paid` /
/// `overpaid`); `None` otherwise, also when it is settled without any
/// payment (a payable of 0).
pub fn paid_date(
    doc_type: DocType,
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

pub fn out_row(s: &Source) -> anyhow::Result<OutRow> {
    let doc = s.doc;
    let Kind {
        direction,
        issued,
        doc_type,
        vat_mode,
    } = Kind::of(doc);
    let is_czk = is_local(&doc.currency);
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
        paid_date: paid_date(doc.doc_type, doc.status, doc.paid, doc.payable, s.payments),
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
