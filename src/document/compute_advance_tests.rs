//! Totals with deducted advances (`advance` lines).

use rust_decimal::Decimal;
use uuid::Uuid;

use super::*;
use crate::document::line::{AdvanceData, ItemData};

fn dec(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

fn item(price: &str, rate: &str) -> LineData {
    LineData::Item(ItemData {
        description: "x".into(),
        quantity: Decimal::ONE,
        unit: None,
        unit_price: dec(price),
        discount_pct: Decimal::ZERO,
        vat_rate: dec(rate),
    })
}

fn row(rate: &str, base: &str, vat: &str, czk: Option<(&str, &str)>) -> AdvanceRow {
    AdvanceRow {
        vat_rate: dec(rate),
        base: dec(base),
        vat: dec(vat),
        base_czk: czk.map(|c| dec(c.0)),
        vat_czk: czk.map(|c| dec(c.1)),
    }
}

fn advance(rows: Vec<AdvanceRow>) -> LineData {
    LineData::Advance(AdvanceData {
        document_id: Uuid::nil(),
        description: "Odpočet zálohy".into(),
        recap: rows,
    })
}

fn params(mode: VatMode, rate: Option<&str>) -> Params {
    Params {
        vat_mode: mode,
        is_czk: rate.is_none(),
        exchange_rate: rate.map(dec),
        rounding: RoundingMode::None,
    }
}

fn eval(lines: &[LineData], p: Params) -> Totals {
    evaluate(lines, p).expect("valid").totals
}

#[test]
fn deducts_ddpp_per_rate() {
    // DDPP of 1000 incl. 21 %: base 826.45, vat 173.55.
    let lines = vec![
        item("1000", "21"),
        item("100", "12"),
        advance(vec![row("21", "826.45", "173.55", None)]),
    ];
    let t = eval(&lines, params(VatMode::Standard, None));
    let r21 = &t.recap[0];
    // vat = round2(1000 × 21 %) − 173.55 = 36.45
    assert_eq!((r21.base, r21.vat), (dec("173.55"), dec("36.45")));
    assert_eq!((t.recap[1].base, t.recap[1].vat), (dec("100"), dec("12")));
    assert_eq!(t.total, dec("322.00"));
    assert_eq!(t.payable, dec("322.00"));
}

#[test]
fn advance_only_rate_and_negative_payable() {
    let lines = vec![
        item("100", "21"),
        advance(vec![
            row("21", "165.29", "34.71", None),
            row("12", "89.29", "10.71", None),
        ]),
    ];
    let t = eval(&lines, params(VatMode::Standard, None));
    assert_eq!(t.recap.len(), 2);
    assert_eq!(
        (t.recap[0].base, t.recap[0].vat),
        (dec("-65.29"), dec("-13.71"))
    );
    assert_eq!(
        (t.recap[1].base, t.recap[1].vat),
        (dec("-89.29"), dec("-10.71"))
    );
    assert_eq!(t.payable, dec("-179.00"));
    let rounded = Params {
        rounding: RoundingMode::Unit,
        ..params(VatMode::Standard, None)
    };
    let lines = vec![
        item("100.4", "0"),
        advance(vec![row("0", "200", "0", None)]),
    ];
    let t = eval(&lines, rounded);
    assert_eq!(
        (t.total, t.payable, t.rounding),
        (dec("-99.60"), dec("-100"), dec("-0.40"))
    );
}

#[test]
fn non_charging_modes_deduct_no_vat() {
    let lines = vec![
        item("1000", "21"),
        advance(vec![row("21", "826.45", "173.55", None)]),
    ];
    let t = eval(&lines, params(VatMode::ReverseCharge, None));
    assert_eq!((t.recap[0].base, t.recap[0].vat), (dec("173.55"), dec("0")));
    let t = eval(
        &[item("1000", "0"), advance(vec![row("0", "400", "0", None)])],
        params(VatMode::NonPayer, None),
    );
    assert_eq!(t.payable, dec("600.00"));
}

#[test]
fn czk_uses_each_advances_own_amounts() {
    // Invoice at 25: items 100 EUR @21 % → 2500 / 525 CZK. The DDPP was at 24.
    let lines = vec![
        item("100", "21"),
        advance(vec![row("21", "50", "10.5", Some(("1200", "252")))]),
    ];
    let t = eval(&lines, params(VatMode::Standard, Some("25")));
    let r = &t.recap[0];
    assert_eq!((r.base, r.vat), (dec("50"), dec("10.50")));
    assert_eq!(
        (r.base_czk, r.vat_czk),
        (Some(dec("1300")), Some(dec("273")))
    );
    assert_eq!(t.total_czk, Some(dec("1512.50")));
    // Without own CZK amounts the advance converts at the invoice's rate.
    let lines = vec![
        item("100", "21"),
        advance(vec![row("21", "50", "10.5", None)]),
    ];
    let r = &eval(&lines, params(VatMode::Standard, Some("25"))).recap[0];
    assert_eq!(
        (r.base_czk, r.vat_czk),
        (Some(dec("1250")), Some(dec("262.50")))
    );
}

#[test]
fn advance_lines_cannot_be_subtotal_members() {
    let lines = vec![
        advance(vec![row("21", "1", "0.21", None)]),
        LineData::Subtotal {
            description: "s".into(),
            refs: vec![1],
            collapse: false,
        },
    ];
    let mut e = FieldErrors::new();
    e.add("lines.1.refs", "invalid");
    assert_eq!(evaluate(&lines, params(VatMode::Standard, None)), Err(e));
}
