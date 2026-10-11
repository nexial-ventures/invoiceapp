//! Pure rules of corrections: the credit-note cap raised by debit notes, the
//! exact VAT of a full DDPP correction, and DDPP amounts net of corrections.

use rust_decimal::Decimal;

use super::compute::{self, Overflow, Params, RecapRow, Totals};
use super::line::AdvanceRow;

impl From<RecapRow> for AdvanceRow {
    fn from(r: RecapRow) -> Self {
        AdvanceRow {
            vat_rate: r.vat_rate,
            base: r.base,
            vat: r.vat,
            base_czk: r.base_czk,
            vat_czk: r.vat_czk,
        }
    }
}

/// `(rate, base)` summed per rate (first-seen order); `None` on overflow.
pub fn sum_bases(
    rows: impl IntoIterator<Item = (Decimal, Decimal)>,
) -> Option<Vec<(Decimal, Decimal)>> {
    let mut sums: Vec<(Decimal, Decimal)> = Vec::new();
    for (rate, base) in rows {
        let rate = rate.normalize();
        match sums.iter_mut().find(|(r, _)| *r == rate) {
            Some((_, s)) => *s = s.checked_add(base)?,
            None => sums.push((rate, base)),
        }
    }
    Some(sums)
}

/// Per rate: the four amounts summed over `rows` with that rate.
fn sum_rows(rows: &[AdvanceRow], rate: Decimal) -> Option<AdvanceRow> {
    let mut out = AdvanceRow {
        vat_rate: rate,
        base: Decimal::ZERO,
        vat: Decimal::ZERO,
        base_czk: None,
        vat_czk: None,
    };
    let add_opt = |a: Option<Decimal>, b: Option<Decimal>| match (a, b) {
        (a, None) => Some(a),
        (None, Some(b)) => Some(Some(b)),
        (Some(a), Some(b)) => a.checked_add(b).map(Some),
    };
    for r in rows.iter().filter(|r| r.vat_rate == rate) {
        out.base = out.base.checked_add(r.base)?;
        out.vat = out.vat.checked_add(r.vat)?;
        out.base_czk = add_opt(out.base_czk, r.base_czk)?;
        out.vat_czk = add_opt(out.vat_czk, r.vat_czk)?;
    }
    Some(out)
}

fn sub_opt(a: Option<Decimal>, b: Option<Decimal>) -> Option<Option<Decimal>> {
    match (a, b) {
        (Some(a), b) => a.checked_sub(b.unwrap_or_default()).map(Some),
        (None, _) => Some(None),
    }
}

/// The DDPP's recap minus its issued corrections, per rate (all four
/// amounts). Rates netting to zero are dropped, so an empty result means the
/// DDPP is fully corrected. `None` on overflow.
pub fn net_recap(ddpp: &[AdvanceRow], corrections: &[AdvanceRow]) -> Option<Vec<AdvanceRow>> {
    let mut out = Vec::with_capacity(ddpp.len());
    for d in ddpp {
        let c = sum_rows(corrections, d.vat_rate)?;
        let row = AdvanceRow {
            vat_rate: d.vat_rate,
            base: d.base.checked_sub(c.base)?,
            vat: d.vat.checked_sub(c.vat)?,
            base_czk: sub_opt(d.base_czk, c.base_czk)?,
            vat_czk: sub_opt(d.vat_czk, c.vat_czk)?,
        };
        let zero = |x: Option<Decimal>| x.is_none_or(|x| x.is_zero());
        if !(row.base.is_zero() && row.vat.is_zero() && zero(row.base_czk) && zero(row.vat_czk)) {
            out.push(row);
        }
    }
    Some(out)
}

/// What the exact-VAT rule of a native DDPP correction needs: the DDPP's
/// recap and what its other issued corrections already credited.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExactBasis {
    pub ddpp: Vec<AdvanceRow>,
    pub credited: Vec<AdvanceRow>,
}

impl ExactBasis {
    /// [`exact_vat`] of `own` against this basis.
    pub fn apply(&self, own: &mut Totals, p: Params) -> Result<(), Overflow> {
        exact_vat(own, &self.ddpp, &self.credited, p)
    }
}

/// Exact VAT of a DDPP correction: for every rate where the credited base —
/// the other issued corrections (`credited`) plus this one — reaches the
/// DDPP's base, this note's VAT (and CZK amounts) are what is left of the
/// DDPP's instead of `round2(base × rate)`, so a fully corrected DDPP nets to
/// exactly zero. Other rates and non-VAT modes are left as computed.
pub fn exact_vat(
    own: &mut Totals,
    ddpp: &[AdvanceRow],
    credited: &[AdvanceRow],
    p: Params,
) -> Result<(), Overflow> {
    if !p.vat_mode.charges_vat() {
        return Ok(());
    }
    let mut changed = false;
    for row in &mut own.recap {
        let Some(d) = ddpp.iter().find(|d| d.vat_rate == row.vat_rate) else {
            continue;
        };
        let c = sum_rows(credited, row.vat_rate).ok_or(Overflow::Lines)?;
        if c.base.checked_add(row.base) != Some(d.base) {
            continue;
        }
        row.vat = compute::fits(d.vat.checked_sub(c.vat).ok_or(Overflow::Lines)?)
            .ok_or(Overflow::Lines)?;
        if row.base_czk.is_some() && d.base_czk.is_some() {
            let left = |a, b| sub_opt(a, b).flatten().ok_or(Overflow::ExchangeRate);
            row.base_czk = Some(left(d.base_czk, c.base_czk)?);
            row.vat_czk = Some(left(d.vat_czk, c.vat_czk)?);
        }
        changed = true;
    }
    if changed {
        *own = compute::summarize(std::mem::take(&mut own.recap), p)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::line::{RoundingMode, VatMode};

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    fn row(rate: &str, base: &str, vat: &str, czk: Option<(&str, &str)>) -> AdvanceRow {
        AdvanceRow {
            vat_rate: d(rate),
            base: d(base),
            vat: d(vat),
            base_czk: czk.map(|c| d(c.0)),
            vat_czk: czk.map(|c| d(c.1)),
        }
    }

    fn params(rate: Option<&str>) -> Params {
        Params {
            vat_mode: VatMode::Standard,
            is_czk: rate.is_none(),
            exchange_rate: rate.map(d),
            rounding: RoundingMode::None,
        }
    }

    /// Totals as compute would produce them for item bases per rate.
    fn computed(rows: &[(&str, &str)], p: Params) -> Totals {
        compute::totals(rows.iter().map(|(r, b)| (d(r), d(b))), p).expect("totals")
    }

    #[test]
    fn sums_bases_per_rate() {
        let s = sum_bases([(d("21"), d("10")), (d("21.00"), d("5")), (d("12"), d("1"))]);
        assert_eq!(s, Some(vec![(d("21"), d("15")), (d("12"), d("1"))]));
        assert_eq!(sum_bases([(d("1"), Decimal::MAX), (d("1"), d("1"))]), None);
    }

    #[test]
    fn full_correction_takes_the_ddpp_vat() {
        // DDPP: VAT from above, 564.98 × 21 % would be 118.65.
        let ddpp = [
            row("21", "564.98", "118.64", None),
            row("12", "282.48", "33.90", None),
        ];
        let mut own = computed(&[("21", "564.98"), ("12", "282.48")], params(None));
        assert_eq!(own.recap[0].vat, d("118.65"));
        exact_vat(&mut own, &ddpp, &[], params(None)).expect("fits");
        assert_eq!(
            (own.recap[0].vat, own.recap[1].vat),
            (d("118.64"), d("33.90"))
        );
        assert_eq!(
            (own.vat, own.total, own.payable),
            (d("152.54"), d("1000.00"), d("1000.00"))
        );
        assert!(
            net_recap(
                &ddpp,
                &[own.recap[0].clone().into(), own.recap[1].clone().into()]
            )
            .expect("fits")
            .is_empty()
        );
    }

    #[test]
    fn partial_then_rest_nets_to_zero() {
        let ddpp = [row("21", "564.98", "118.64", Some(("14124.50", "2966.00")))];
        let p = params(Some("25"));
        let mut first = computed(&[("21", "100")], p);
        exact_vat(&mut first, &ddpp, &[], p).expect("fits");
        assert_eq!(first.recap[0].vat, d("21.00"), "partial: computed normally");
        let credited: Vec<AdvanceRow> = first.recap.iter().cloned().map(Into::into).collect();
        let mut rest = computed(&[("21", "464.98")], p);
        exact_vat(&mut rest, &ddpp, &credited, p).expect("fits");
        assert_eq!(rest.recap[0].vat, d("97.64"));
        assert_eq!(rest.recap[0].base_czk, Some(d("11624.50")));
        assert_eq!(rest.recap[0].vat_czk, Some(d("2441.00")));
        assert_eq!(rest.total_czk, Some(d("14065.50")));
        let mut all = credited.clone();
        all.extend(rest.recap.iter().cloned().map(AdvanceRow::from));
        assert_eq!(net_recap(&ddpp, &all), Some(vec![]));
    }

    #[test]
    fn other_rates_and_modes_untouched() {
        let ddpp = [row("21", "100", "20", None)];
        let mut own = computed(&[("12", "100"), ("21", "50")], params(None));
        let before = own.clone();
        exact_vat(&mut own, &ddpp, &[], params(None)).expect("fits");
        assert_eq!(own, before);
        let mut exempt = params(None);
        exempt.vat_mode = VatMode::Exempt;
        let mut own = computed(&[("21", "100")], exempt);
        let before = own.clone();
        exact_vat(&mut own, &ddpp, &[], exempt).expect("fits");
        assert_eq!(own, before);
    }

    #[test]
    fn net_keeps_partially_corrected_rates() {
        let ddpp = [row("21", "100", "21", None), row("12", "50", "6", None)];
        let net = net_recap(
            &ddpp,
            &[row("21", "40", "8.40", None), row("12", "50", "6", None)],
        );
        assert_eq!(net, Some(vec![row("21", "60", "12.60", None)]));
        assert_eq!(net_recap(&ddpp, &[]), Some(ddpp.to_vec()));
    }
}
