//! Pure money computation for documents: line bases, the per-rate VAT recap
//! (§37 ZDPH — VAT per rate, not per line), rounding (`rounding.rs`) and CZK conversion.
//! Authoritative: the stored totals and the compute endpoint both come from here.
//!
//! All arithmetic is checked: `Decimal`'s operators panic on overflow, and
//! huge inputs must end as a 422, never a crashed request.

use rust_decimal::{Decimal, RoundingStrategy};

use super::line::{AdvanceRow, LineData, MAX_AMOUNT, RoundingMode, VatMode};
use super::subtotals;
use crate::error::FieldErrors;

pub fn round2(x: Decimal) -> Decimal {
    let mut r = x.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    r.rescale(2);
    r
}

/// `round2(quantity × unitPrice × (1 − discount/100))`, half away from zero;
/// `None` on overflow or when the base would not fit a money column.
pub fn item_base(quantity: Decimal, unit_price: Decimal, discount_pct: Decimal) -> Option<Decimal> {
    let base = quantity
        .checked_mul(unit_price)?
        .checked_mul(Decimal::ONE_HUNDRED.checked_sub(discount_pct)?)?
        .checked_div(Decimal::ONE_HUNDRED)?;
    fits(round2(base))
}

/// `Some(x)` when `x` fits a `numeric(18,2)` column.
pub fn fits(x: Decimal) -> Option<Decimal> {
    (x.abs() < MAX_AMOUNT).then_some(x)
}

/// Which input made the totals unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Lines,
    ExchangeRate,
}

impl Overflow {
    pub fn field(self) -> &'static str {
        match self {
            Overflow::Lines => "lines",
            Overflow::ExchangeRate => "exchangeRate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecapRow {
    pub vat_rate: Decimal,
    pub base: Decimal,
    pub vat: Decimal,
    pub base_czk: Option<Decimal>,
    pub vat_czk: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Totals {
    /// Sorted by rate, highest first.
    pub recap: Vec<RecapRow>,
    pub base: Decimal,
    pub vat: Decimal,
    pub total: Decimal,
    pub rounding: Decimal,
    pub payable: Decimal,
    pub total_czk: Option<Decimal>,
}

#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub vat_mode: VatMode,
    pub is_czk: bool,
    /// CZK per unit of a foreign currency, when known.
    pub exchange_rate: Option<Decimal>,
    pub rounding: RoundingMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluated {
    /// `(vat_rate, base)` per line; `None` for text lines.
    pub lines: subtotals::Resolved,
    pub totals: Totals,
}

/// Validate the computation rules and compute everything. Field errors use
/// `lines.<index>.<field>` with the 0-based index of the line; amounts too
/// large to store are `lines: invalid` (or `exchangeRate: invalid` when only
/// the CZK conversion overflows).
pub fn evaluate(lines: &[LineData], p: Params) -> Result<Evaluated, FieldErrors> {
    let mut e = FieldErrors::new();
    let mut bases = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let base = match line {
            LineData::Item(item) => {
                if p.vat_mode == VatMode::NonPayer && !item.vat_rate.is_zero() {
                    e.add(&format!("lines.{i}.vatRate"), "invalid");
                }
                item_base(item.quantity, item.unit_price, item.discount_pct).unwrap_or_else(|| {
                    e.add("lines", "invalid");
                    Decimal::ZERO
                })
            }
            _ => Decimal::ZERO,
        };
        bases.push(base);
    }
    let resolved = subtotals::resolve(lines, &bases);
    if let Err(i) = &resolved {
        e.add(&format!("lines.{i}.refs"), "invalid");
    }
    let resolved = match resolved {
        Ok(r) if e.is_empty() => r,
        _ => return Err(e),
    };
    let items = lines.iter().zip(&bases).filter_map(|(l, b)| match l {
        LineData::Item(item) => Some((item.vat_rate, *b)),
        _ => None,
    });
    let advances: Vec<&AdvanceRow> = lines
        .iter()
        .filter_map(|l| match l {
            LineData::Advance(a) => Some(a.recap.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    let totals = totals_with(items, &advances, p).map_err(|o| {
        e.add(o.field(), "invalid");
        e.clone()
    })?;
    Ok(Evaluated {
        lines: resolved,
        totals,
    })
}

/// Recap + totals from `(vat_rate, base)` of the item lines.
pub fn totals(
    items: impl IntoIterator<Item = (Decimal, Decimal)>,
    p: Params,
) -> Result<Totals, Overflow> {
    totals_with(items, &[], p)
}

/// Per-rate sums: item bases and the deducted advance amounts.
#[derive(Default)]
struct Group {
    rate: Decimal,
    items: Decimal,
    adv_base: Decimal,
    adv_vat: Decimal,
    adv_base_czk: Decimal,
    adv_vat_czk: Decimal,
}

/// Recap + totals with deducted advances: per rate
/// `base = Σ item base − Σ advance base`,
/// `vat = round2(Σ item base × rate/100) − Σ advance vat` (VAT-charging modes
/// only), CZK: the item part at the document's rate minus each advance's own
/// CZK amounts. Without advances this is exactly the 1b computation.
pub fn totals_with(
    items: impl IntoIterator<Item = (Decimal, Decimal)>,
    advances: &[&AdvanceRow],
    p: Params,
) -> Result<Totals, Overflow> {
    use Overflow::{ExchangeRate, Lines};
    let fx = if p.is_czk { None } else { p.exchange_rate };
    let czk = |x: Decimal| -> Result<Option<Decimal>, Overflow> {
        fx.map(|r| {
            x.checked_mul(r)
                .map(round2)
                .and_then(fits)
                .ok_or(ExchangeRate)
        })
        .transpose()
    };
    let add = |a: Decimal, b: Decimal| a.checked_add(b).ok_or(Lines);
    let mut groups: Vec<Group> = Vec::new();
    fn group(groups: &mut Vec<Group>, rate: Decimal) -> &mut Group {
        let i = match groups.iter().position(|g| g.rate == rate) {
            Some(i) => i,
            None => {
                groups.push(Group {
                    rate,
                    ..Default::default()
                });
                groups.len() - 1
            }
        };
        &mut groups[i]
    }
    for (rate, base) in items {
        let g = group(&mut groups, rate);
        g.items = add(g.items, base)?;
    }
    for a in advances {
        let base_czk = match a.base_czk {
            Some(c) => c,
            None => czk(a.base)?.unwrap_or_default(),
        };
        let vat_czk = match a.vat_czk {
            Some(c) => c,
            None => czk(a.vat)?.unwrap_or_default(),
        };
        let g = group(&mut groups, a.vat_rate);
        g.adv_base = add(g.adv_base, a.base)?;
        g.adv_vat = add(g.adv_vat, a.vat)?;
        g.adv_base_czk = add(g.adv_base_czk, base_czk)?;
        g.adv_vat_czk = add(g.adv_vat_czk, vat_czk)?;
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.rate));
    let sub = |a: Decimal, b: Decimal| a.checked_sub(b).map(round2).and_then(fits);
    let mut recap = Vec::with_capacity(groups.len());
    for g in groups {
        let item_base = fits(round2(g.items)).ok_or(Lines)?;
        let item_vat = if p.vat_mode.charges_vat() {
            round2(item_base.checked_mul(g.rate).ok_or(Lines)? / Decimal::ONE_HUNDRED)
        } else {
            Decimal::ZERO
        };
        let adv_vat = if p.vat_mode.charges_vat() {
            g.adv_vat
        } else {
            Decimal::ZERO
        };
        let base = sub(item_base, g.adv_base).ok_or(Lines)?;
        let vat = sub(item_vat, adv_vat).ok_or(Lines)?;
        let adv_vat_czk = if p.vat_mode.charges_vat() {
            g.adv_vat_czk
        } else {
            Decimal::ZERO
        };
        let base_czk = czk(item_base)?
            .map(|c| sub(c, g.adv_base_czk).ok_or(ExchangeRate))
            .transpose()?;
        let vat_czk = czk(item_vat)?
            .map(|c| sub(c, adv_vat_czk).ok_or(ExchangeRate))
            .transpose()?;
        recap.push(RecapRow {
            vat_rate: g.rate.normalize(),
            base,
            vat,
            base_czk,
            vat_czk,
        });
    }
    summarize(recap, p)
}

/// Document totals from a finished recap: Σ base, Σ vat, total, rounding,
/// payable and its CZK amount.
pub fn summarize(recap: Vec<RecapRow>, p: Params) -> Result<Totals, Overflow> {
    use Overflow::{ExchangeRate, Lines};
    let fx = if p.is_czk { None } else { p.exchange_rate };
    let add = |a: Decimal, b: Decimal| a.checked_add(b).ok_or(Lines);
    let (mut base_sum, mut vat_sum) = (Decimal::ZERO, Decimal::ZERO);
    for r in &recap {
        base_sum = add(base_sum, r.base)?;
        vat_sum = add(vat_sum, r.vat)?;
    }
    let base = fits(round2(base_sum)).ok_or(Lines)?;
    let vat = fits(round2(vat_sum)).ok_or(Lines)?;
    let total = fits(round2(add(base, vat)?)).ok_or(Lines)?;
    let payable = p.rounding.apply(total).and_then(fits).ok_or(Lines)?;
    Ok(Totals {
        recap,
        base,
        vat,
        total,
        rounding: round2(payable - total),
        payable,
        total_czk: fx
            .map(|r| {
                payable
                    .checked_mul(r)
                    .map(round2)
                    .and_then(fits)
                    .ok_or(ExchangeRate)
            })
            .transpose()?,
    })
}

#[cfg(test)]
#[path = "compute_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "compute_advance_tests.rs"]
mod advance_tests;
