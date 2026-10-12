//! [`Parsed`] → what gets stored: direction, sign, VAT mode, lines, recap
//! and totals (copied, never recomputed), payment fields. Pure.

use rust_decimal::Decimal;

use super::model::{Line, Money, Parsed};
use super::parse::{Code, INVALID_AMOUNT};
use crate::currency::is_local;
use crate::document::compute::{RecapRow, Totals, item_base, round2};
use crate::document::doc_type::DocType;
use crate::document::line::Direction;
use crate::document::line::{ItemData, LineData, PaymentMethod, VatMode};
use crate::document::snapshot::BankSnapshot;
pub use crate::import::model::Plan;
use crate::import::model::{ContactRule, IssuedBank};

pub const FOREIGN: Code = "foreign";
pub const AMBIGUOUS: Code = "ambiguous";
pub const RATE_FROM_CNB: Code = "rate_from_cnb";

fn ico_eq(a: Option<&str>, company: &str) -> bool {
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    a.is_some_and(|a| !company.is_empty() && strip(a) == strip(company))
}

fn direction(p: &Parsed, company_ico: Option<&str>) -> Result<Direction, Code> {
    let company = company_ico.unwrap_or_default();
    let supplier = ico_eq(p.supplier.ico.as_deref(), company);
    let customer = ico_eq(p.customer.as_ref().and_then(|c| c.ico.as_deref()), company);
    match (supplier, customer) {
        (true, true) => Err(AMBIGUOUS),
        (true, false) => Ok(Direction::Issued),
        (false, true) => Ok(Direction::Received),
        (false, false) => Err(FOREIGN),
    }
}

fn vat_mode(p: &Parsed, direction: Direction) -> VatMode {
    let note_rc = p
        .note
        .as_deref()
        .is_some_and(|n| n.to_lowercase().contains("přenesen"));
    let rc = p
        .recap
        .iter()
        .any(|r| r.reverse_charge || (r.vat_applicable == Some(false) && !r.rate.is_zero()));
    if rc || note_rc {
        return VatMode::ReverseCharge;
    }
    if p.recap
        .iter()
        .all(|r| r.rate.is_zero() && r.vat_applicable == Some(false))
    {
        return if direction == Direction::Issued {
            VatMode::NonPayer
        } else {
            VatMode::Exempt
        };
    }
    // Our own export of an exempt document: VAT applicable, no VAT on any
    // row although some row's base × rate would give some.
    let exempt = p
        .recap
        .iter()
        .all(|r| r.vat_applicable == Some(true) && r.vat.doc.is_zero())
        && p.recap
            .iter()
            .any(|r| !round2(r.base.doc * r.rate / Decimal::ONE_HUNDRED).is_zero());
    if exempt {
        VatMode::Exempt
    } else {
        VatMode::Standard
    }
}

/// An item (or a text line when it carries no amounts). The unit price is in
/// the document currency: `UnitPrice` for CZK, else base ÷ quantity.
fn line(l: &Line, foreign: bool, k: Decimal) -> Result<LineData, Code> {
    let empty =
        l.quantity.is_none_or(|q| q.is_zero()) && l.base.doc.is_zero() && l.unit_price.is_zero();
    if empty {
        return Ok(LineData::Text {
            description: l.description.clone(),
        });
    }
    let quantity = l.quantity.unwrap_or(Decimal::ONE).round_dp(4);
    let base = l.base.doc * k;
    let (unit_price, discount) = if foreign {
        let unit = if quantity.is_zero() {
            base
        } else {
            base.checked_div(quantity).ok_or(INVALID_AMOUNT)?
        };
        (unit, Decimal::ZERO)
    } else {
        // `LineExtensionAmountBeforeDiscount` above the base is a discount.
        let discount = match l.before_discount.map(|b| b * k) {
            Some(before) if !before.is_zero() && before != base => round2(
                (Decimal::ONE - base.checked_div(before).ok_or(INVALID_AMOUNT)?)
                    * Decimal::ONE_HUNDRED,
            ),
            _ => Decimal::ZERO,
        };
        if discount < Decimal::ZERO || discount > Decimal::ONE_HUNDRED {
            (l.unit_price * k, Decimal::ZERO)
        } else {
            (l.unit_price * k, discount)
        }
    };
    let unit_price = unit_price.round_dp(4);
    // Lines are re-evaluated on every read: their base must fit a money column.
    if item_base(quantity, unit_price, discount).is_none() {
        return Err(INVALID_AMOUNT);
    }
    Ok(LineData::Item(ItemData {
        description: l.description.clone(),
        quantity,
        unit: l.unit.clone(),
        unit_price: unit_price.normalize(),
        discount_pct: discount.normalize(),
        vat_rate: l.rate,
    }))
}

fn czk(m: Money, k: Decimal) -> Option<Decimal> {
    m.czk.map(|c| round2(c * k))
}

fn totals(p: &Parsed, k: Decimal) -> Totals {
    let mut recap: Vec<RecapRow> = p
        .recap
        .iter()
        .map(|r| RecapRow {
            vat_rate: r.rate,
            base: round2(r.base.doc * k),
            vat: round2(r.vat.doc * k),
            base_czk: czk(r.base, k),
            vat_czk: czk(r.vat, k),
        })
        .collect();
    // Non-taxed deposits (`PaidDepositsAmount`) are VAT-free, so they come off
    // the 0 % row — as an `advance` line of a non-payer deducts a proforma —
    // keeping `payable = total + rounding`.
    let paid = p.totals.paid_deposits;
    if !paid.doc.is_zero() {
        let i = match recap.iter().position(|r| r.vat_rate.is_zero()) {
            Some(i) => i,
            None => {
                recap.push(RecapRow {
                    vat_rate: Decimal::ZERO,
                    base: round2(Decimal::ZERO),
                    vat: round2(Decimal::ZERO),
                    base_czk: paid.czk.map(|_| round2(Decimal::ZERO)),
                    vat_czk: paid.czk.map(|_| round2(Decimal::ZERO)),
                });
                recap.len() - 1
            }
        };
        let row = &mut recap[i];
        row.base = round2(row.base - paid.doc * k);
        row.base_czk = row.base_czk.zip(czk(paid, k)).map(|(b, d)| round2(b - d));
    }
    recap.sort_by_key(|r| std::cmp::Reverse(r.vat_rate));
    let base = round2((p.totals.base.doc - paid.doc) * k);
    let total = round2((p.totals.total.doc - paid.doc) * k);
    Totals {
        recap,
        base,
        vat: total - base,
        total,
        rounding: round2(p.totals.rounding * k),
        payable: round2(p.totals.payable.doc * k),
        total_czk: czk(p.totals.payable, k),
    }
}

/// Plain digits up to `max`, else dropped.
fn digits(s: Option<&str>, max: usize) -> Option<String> {
    s.filter(|s| s.len() <= max && s.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_string)
}

fn method(code: Option<&str>) -> PaymentMethod {
    match code {
        Some("42") => PaymentMethod::BankTransfer,
        Some("10") => PaymentMethod::Cash,
        Some("48") => PaymentMethod::Card,
        _ => PaymentMethod::Other,
    }
}

pub fn plan(p: Parsed, company_ico: Option<&str>) -> Result<Plan, Code> {
    let direction = direction(&p, company_ico)?;
    // Corrections are positive (ISDOC 6 rule A.6); a negative total is
    // flipped as a whole so mixed-sign lines keep their relation.
    let k = if p.doc_type.sign() < 0 && p.totals.gross < Decimal::ZERO {
        Decimal::NEGATIVE_ONE
    } else {
        Decimal::ONE
    };
    let foreign = !is_local(&p.currency);
    let vat_mode = vat_mode(&p, direction);
    let mut lines = p
        .lines
        .iter()
        .map(|l| line(l, foreign, k))
        .collect::<Result<Vec<_>, _>>()?;
    // A non-payer line carries no rate (the read-time evaluation refuses one).
    if vat_mode == VatMode::NonPayer {
        for l in &mut lines {
            if let LineData::Item(i) = l {
                i.vat_rate = Decimal::ZERO;
            }
        }
    }
    let mut warnings = Vec::new();
    let pay = p.payment.clone().unwrap_or_default();
    if foreign && p.rate.is_none() {
        warnings.push(RATE_FROM_CNB);
    }
    let account = match (&pay.account, &pay.bank_code) {
        (Some(a), Some(b)) => Some(format!("{a}/{b}")),
        (a, _) => a.clone(),
    };
    let iban = pay.iban.as_deref().map(crate::validation::normalize_iban);
    let bank = (account.is_some() || iban.is_some()).then(|| BankSnapshot {
        account_number: account,
        iban,
        bic: pay.bic.clone(),
    });
    let tax_point_date = match p.doc_type {
        DocType::Proforma => None,
        _ => Some(p.tax_point_date.unwrap_or(p.issue_date)),
    };
    let due_date = match (direction, p.doc_type, pay.due_date) {
        (Direction::Received, DocType::AdvanceTaxDoc, d) => d,
        (_, _, d) => Some(d.unwrap_or(p.issue_date)),
    };
    Ok(Plan {
        direction,
        vat_mode,
        totals: totals(&p, k),
        gross: round2(p.totals.gross * k),
        doc_type: p.doc_type,
        number: p.number,
        vat_payer: Some(p.vat_applicable),
        issue_date: p.issue_date,
        tax_point_date,
        due_date,
        currency: p.currency,
        rate: p.rate,
        lines,
        payment_method: method(pay.code.as_deref()),
        variable_symbol: digits(pay.variable_symbol.as_deref(), 10),
        constant_symbol: digits(pay.constant_symbol.as_deref(), 4),
        bank,
        note: p.note,
        locale: "cs".into(),
        received_date: None,
        original_ref: p.original_ref,
        supplier: p.supplier,
        customer: p.customer,
        warnings,
        contact_rule: ContactRule::IcoOrName,
        issued_bank: IssuedBank::Payment,
    })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
