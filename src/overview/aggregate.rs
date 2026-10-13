//! The period figures of the overview: totals per direction, VAT per rate,
//! the monthly series and the categories, from the period's documents in
//! CZK. Pure.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::dto::{Categories, CategoryAmount, Month, Totals, Vat, VatRow};
use super::period::{Chart, month_key, overlaps};
use crate::document::doc_type::DocType;
use crate::document::line::{Direction, VatMode};

/// A non-proforma, issued-status document with a tax date, in CZK.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub direction: Direction,
    pub doc_type: DocType,
    pub vat_mode: VatMode,
    pub vat_deductible: bool,
    pub tax_date: NaiveDate,
    pub category_id: Option<Uuid>,
    /// `(rate, base, VAT)`, positive as stored.
    pub recap: Vec<(Decimal, Decimal, Decimal)>,
    /// `total + rounding`, positive as stored.
    pub total: Decimal,
}

impl Doc {
    fn sign(&self) -> Decimal {
        Decimal::from(self.doc_type.sign())
    }

    /// Received VAT that cannot be deducted: it is part of the expense.
    fn non_deductible(&self) -> bool {
        self.direction == Direction::Received
            && self.vat_mode == VatMode::Standard
            && !self.vat_deductible
    }

    /// `(base, VAT)`, signed.
    fn base_vat(&self) -> (Decimal, Decimal) {
        let (b, v) = self
            .recap
            .iter()
            .fold((Decimal::ZERO, Decimal::ZERO), |(b, v), r| {
                (b + r.1, v + r.2)
            });
        (b * self.sign(), v * self.sign())
    }

    /// The amount by the basis: a VAT payer's base (+ non-deductible VAT), a
    /// non-payer's total; signed.
    pub fn amount(&self, vat_payer: bool) -> Decimal {
        if !vat_payer {
            return self.total * self.sign();
        }
        let (base, vat) = self.base_vat();
        match self.non_deductible() {
            true => base + vat,
            false => base,
        }
    }
}

/// What the period's documents add up to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Figures {
    pub income: Totals,
    pub expense: Totals,
    pub vat: Vat,
    pub months: Vec<Month>,
    pub categories: Categories,
}

fn add_totals(t: &mut Totals, d: &Doc, vat_payer: bool) {
    let (base, vat) = d.base_vat();
    t.count += 1;
    t.amount += d.amount(vat_payer);
    t.base += base;
    t.vat += vat;
    t.total += d.total * d.sign();
}

fn add_rows(rows: &mut Vec<VatRow>, d: &Doc) {
    let s = d.sign();
    for &(rate, base, vat) in &d.recap {
        let rate = rate.normalize();
        let i = match rows.iter().position(|r| r.rate == rate) {
            Some(i) => i,
            None => {
                rows.push(VatRow {
                    rate,
                    ..VatRow::default()
                });
                rows.len() - 1
            }
        };
        rows[i].base += base * s;
        rows[i].vat += vat * s;
    }
}

fn add_vat(v: &mut Vat, d: &Doc) {
    let issued = d.direction == Direction::Issued;
    match d.vat_mode {
        VatMode::ReverseCharge => {
            let (base, _) = d.base_vat();
            match issued {
                true => v.reverse_charge_issued += base,
                false => v.reverse_charge_received += base,
            }
        }
        VatMode::Standard if issued => add_rows(&mut v.output, d),
        VatMode::Standard if d.non_deductible() => {
            let (base, vat) = d.base_vat();
            v.non_deductible.base += base;
            v.non_deductible.vat += vat;
        }
        VatMode::Standard => add_rows(&mut v.input, d),
        VatMode::Exempt | VatMode::NonPayer => {}
    }
}

fn finish_vat(v: &mut Vat) {
    for rows in [&mut v.output, &mut v.input] {
        rows.sort_by_key(|r| std::cmp::Reverse(r.rate));
    }
    v.output_vat = v.output.iter().map(|r| r.vat).sum();
    v.input_vat = v.input.iter().map(|r| r.vat).sum();
    v.result = v.output_vat - v.input_vat;
}

fn first_of_month(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

fn months(
    docs: &[Doc],
    chart: &Chart,
    from: NaiveDate,
    to: NaiveDate,
    vat_payer: bool,
) -> Vec<Month> {
    let mut sums: BTreeMap<NaiveDate, (Decimal, Decimal)> = chart
        .months
        .iter()
        .map(|m| (*m, (Decimal::ZERO, Decimal::ZERO)))
        .collect();
    let in_chart = docs
        .iter()
        .filter(|d| (chart.from..=chart.to).contains(&d.tax_date));
    for d in in_chart {
        if let Some((income, expense)) = sums.get_mut(&first_of_month(d.tax_date)) {
            match d.direction {
                Direction::Issued => *income += d.amount(vat_payer),
                Direction::Received => *expense += d.amount(vat_payer),
            }
        }
    }
    sums.into_iter()
        .map(|(first, (income, expense))| Month {
            month: month_key(first),
            income,
            expense,
            in_period: overlaps(first, from, to),
        })
        .collect()
}

fn categories(
    docs: &[&Doc],
    direction: Direction,
    names: &HashMap<Uuid, String>,
    vat_payer: bool,
) -> Vec<CategoryAmount> {
    let mut by: Vec<CategoryAmount> = Vec::new();
    for d in docs.iter().filter(|d| d.direction == direction) {
        let i = match by.iter().position(|c| c.id == d.category_id) {
            Some(i) => i,
            None => {
                by.push(CategoryAmount {
                    id: d.category_id,
                    name: d.category_id.and_then(|c| names.get(&c).cloned()),
                    count: 0,
                    amount: Decimal::ZERO,
                });
                by.len() - 1
            }
        };
        by[i].count += 1;
        by[i].amount += d.amount(vat_payer);
    }
    // Largest first; uncategorized last among equals, then by name.
    by.sort_by(|a, b| {
        b.amount
            .cmp(&a.amount)
            .then(a.id.is_none().cmp(&b.id.is_none()))
            .then(a.name.cmp(&b.name))
    });
    by
}

/// The figures of `from..=to` from `docs` (the chart's range, a superset);
/// `names` = category names.
pub fn figures(
    docs: &[Doc],
    from: NaiveDate,
    to: NaiveDate,
    chart: &Chart,
    vat_payer: bool,
    names: &HashMap<Uuid, String>,
) -> Figures {
    let period: Vec<&Doc> = docs
        .iter()
        .filter(|d| (from..=to).contains(&d.tax_date))
        .collect();
    let mut f = Figures::default();
    for d in &period {
        match d.direction {
            Direction::Issued => add_totals(&mut f.income, d, vat_payer),
            Direction::Received => add_totals(&mut f.expense, d, vat_payer),
        }
        add_vat(&mut f.vat, d);
    }
    finish_vat(&mut f.vat);
    f.months = months(docs, chart, from, to, vat_payer);
    f.categories = Categories {
        income: categories(&period, Direction::Issued, names, vat_payer),
        expense: categories(&period, Direction::Received, names, vat_payer),
    };
    f
}

#[cfg(test)]
#[path = "aggregate_tests.rs"]
mod tests;
