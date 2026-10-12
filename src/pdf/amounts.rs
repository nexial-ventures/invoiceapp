//! The amount parts of the payload: lines, VAT recaps and the totals rows.
//! Credit notes are stored positive and print negated.

use std::collections::{HashMap, HashSet};

use anyhow::Context as _;
use rust_decimal::Decimal;

use super::format::{self, Locale};
use super::labels::{self, Labels};
use super::payload::{Input, PdfLine, Recap, RecapRow, TotalRow};
use crate::document::line::VatMode;
use crate::document::wire::dto::Line;

pub struct Amounts {
    pub has_discount: bool,
    pub lines: Vec<PdfLine>,
    pub vat_recap: Option<Recap>,
    pub vat_recap_czk: Option<Recap>,
    pub totals: Vec<TotalRow>,
}

fn add(a: Decimal, b: Decimal) -> anyhow::Result<Decimal> {
    a.checked_add(b).context("PDF amount overflow")
}

fn sub(a: Decimal, b: Decimal) -> anyhow::Result<Decimal> {
    a.checked_sub(b).context("PDF amount overflow")
}

/// Positions hidden by a collapsed subtotal: its members, and the members of
/// any subtotal among them, recursively.
fn hidden(lines: &[Line]) -> HashSet<i32> {
    let subtotals: HashMap<i32, &Vec<i32>> = lines
        .iter()
        .filter_map(|l| match l {
            Line::Subtotal(s) => Some((s.position, &s.refs)),
            _ => None,
        })
        .collect();
    let mut stack: Vec<i32> = lines
        .iter()
        .filter_map(|l| match l {
            Line::Subtotal(s) if s.collapse => Some(s.refs.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect();
    let mut out = HashSet::new();
    while let Some(p) = stack.pop() {
        if out.insert(p)
            && let Some(refs) = subtotals.get(&p)
        {
            stack.extend(refs.iter().copied());
        }
    }
    out
}

struct Fmt<'a> {
    locale: Locale,
    currency: &'a str,
    sign: Decimal,
}

impl Fmt<'_> {
    fn money(&self, x: Decimal) -> String {
        format::money(x * self.sign, self.currency, self.locale)
    }

    fn czk(&self, x: Decimal) -> String {
        format::money(x * self.sign, "CZK", self.locale)
    }
}

fn pdf_lines(i: &Input, f: &Fmt, show_vat: bool) -> Vec<PdfLine> {
    let hidden = hidden(i.lines);
    let rate = |r: Decimal| show_vat.then(|| format::percent(r, i.locale));
    let blank = |kind, description: &str| PdfLine {
        kind,
        description: description.to_string(),
        quantity: None,
        unit_price: None,
        discount: None,
        vat_rate: None,
        base: None,
        strong: false,
    };
    i.lines
        .iter()
        .filter_map(|line| {
            let position = match line {
                Line::Item(l) => l.position,
                Line::Text(l) => l.position,
                Line::Subtotal(l) => l.position,
                Line::Advance(l) => l.position,
            };
            if hidden.contains(&position) {
                return None;
            }
            Some(match line {
                Line::Item(l) => PdfLine {
                    quantity: Some(format::quantity(l.quantity, l.unit.as_deref(), i.locale)),
                    unit_price: Some(f.money(l.unit_price)),
                    discount: (!l.discount_pct.is_zero())
                        .then(|| format::percent(l.discount_pct, i.locale)),
                    vat_rate: rate(l.vat_rate),
                    base: Some(f.money(l.base)),
                    ..blank("item", &l.description)
                },
                Line::Text(l) => blank("text", &l.description),
                Line::Subtotal(l) => PdfLine {
                    vat_rate: rate(l.vat_rate),
                    base: Some(f.money(l.base)),
                    strong: !l.collapse,
                    ..blank("subtotal", &l.description)
                },
                Line::Advance(l) => PdfLine {
                    base: Some(f.money(l.base)),
                    ..blank("advance", &l.description)
                },
            })
        })
        .collect()
}

fn recap_columns(l: &Labels) -> RecapRow {
    RecapRow {
        rate: l.rate.into(),
        base: l.base.into(),
        vat: l.vat.into(),
        total: l.total.into(),
    }
}

fn vat_recap(i: &Input, f: &Fmt, l: &Labels) -> anyhow::Result<Recap> {
    let t = i.totals;
    let rows = t
        .recap
        .iter()
        .map(|r| {
            Ok(RecapRow {
                rate: format::percent(r.vat_rate, i.locale),
                base: f.money(r.base),
                vat: f.money(r.vat),
                total: f.money(add(r.base, r.vat)?),
            })
        })
        .collect::<anyhow::Result<_>>()?;
    Ok(Recap {
        title: l.vat_recap.into(),
        rate_note: None,
        columns: recap_columns(l),
        rows,
        total: RecapRow {
            rate: l.total.into(),
            base: f.money(t.base),
            vat: f.money(t.vat),
            total: f.money(add(t.base, t.vat)?),
        },
    })
}

/// § 37: a foreign-currency document charging VAT also states it in CZK.
fn vat_recap_czk(i: &Input, f: &Fmt, l: &Labels) -> anyhow::Result<Option<Recap>> {
    let Some(rate) = i.rate else {
        return Ok(None);
    };
    if i.currency == "CZK" || i.vat_mode != VatMode::Standard {
        return Ok(None);
    }
    let mut rows = Vec::new();
    let (mut base, mut vat) = (Decimal::ZERO, Decimal::ZERO);
    for r in &i.totals.recap {
        let (Some(b), Some(v)) = (r.base_czk, r.vat_czk) else {
            return Ok(None);
        };
        base = add(base, b)?;
        vat = add(vat, v)?;
        rows.push(RecapRow {
            rate: format::percent(r.vat_rate, i.locale),
            base: f.czk(b),
            vat: f.czk(v),
            total: f.czk(add(b, v)?),
        });
    }
    let cnb_date = rate.cnb_date.map(|d| format::date(d, i.locale));
    Ok(Some(Recap {
        title: l.vat_recap_czk.into(),
        rate_note: Some(labels::rate_note(
            &format::rate(rate.rate, i.locale),
            i.currency,
            cnb_date.as_deref(),
            i.locale,
        )),
        columns: recap_columns(l),
        rows,
        total: RecapRow {
            rate: l.total.into(),
            base: f.czk(base),
            vat: f.czk(vat),
            total: f.czk(add(base, vat)?),
        },
    }))
}

fn total_row(label: &str, value: String, strong: bool) -> TotalRow {
    TotalRow {
        label: label.into(),
        value,
        strong,
    }
}

/// The stored totals are net of advance deductions; the rows show the
/// amounts before the deduction, the deduction, then the payable.
fn totals(i: &Input, f: &Fmt, l: &Labels, show_vat: bool) -> anyhow::Result<Vec<TotalRow>> {
    let t = i.totals;
    if i.doc_type == "advance_tax_doc" {
        return Ok(vec![
            total_row(l.total_excl_vat, f.money(t.base), false),
            total_row(l.vat, f.money(t.vat), false),
            total_row(l.total_incl_vat, f.money(t.total), true),
        ]);
    }
    let (mut adv_base, mut adv_vat, mut has_adv) = (Decimal::ZERO, Decimal::ZERO, false);
    for line in i.lines {
        if let Line::Advance(a) = line {
            has_adv = true;
            adv_base = add(adv_base, a.base)?;
            for r in &a.recap {
                adv_vat = add(adv_vat, r.vat)?;
            }
        }
    }
    let adv_gross = add(adv_base, adv_vat)?;
    let mut rows = if show_vat {
        vec![
            total_row(l.total_excl_vat, f.money(sub(t.base, adv_base)?), false),
            total_row(l.vat, f.money(sub(t.vat, adv_vat)?), false),
            total_row(l.total_incl_vat, f.money(sub(t.total, adv_gross)?), false),
        ]
    } else {
        vec![total_row(l.total, f.money(sub(t.total, adv_gross)?), false)]
    };
    if has_adv {
        rows.push(total_row(l.advances, f.money(adv_gross), false));
    }
    if !t.rounding.is_zero() {
        rows.push(total_row(l.rounding, f.money(t.rounding), false));
    }
    rows.push(total_row(l.payable, f.money(t.payable), true));
    Ok(rows)
}

pub fn build(i: &Input, l: &Labels, show_vat: bool) -> anyhow::Result<Amounts> {
    let f = Fmt {
        locale: i.locale,
        currency: i.currency,
        sign: Decimal::from(crate::document::wire::dto::sign(i.doc_type)),
    };
    let has_discount = i
        .lines
        .iter()
        .any(|l| matches!(l, Line::Item(it) if !it.discount_pct.is_zero()));
    Ok(Amounts {
        has_discount,
        lines: pdf_lines(i, &f, show_vat),
        vat_recap: if show_vat {
            Some(vat_recap(i, &f, l)?)
        } else {
            None
        },
        vat_recap_czk: if show_vat {
            vat_recap_czk(i, &f, l)?
        } else {
            None
        },
        totals: totals(i, &f, l, show_vat)?,
    })
}

#[cfg(test)]
#[path = "amounts_tests.rs"]
mod tests;
