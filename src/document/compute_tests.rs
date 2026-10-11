use rust_decimal::Decimal;

use super::*;
use crate::document::line::ItemData;

fn dec(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

fn ib(q: Decimal, p: Decimal, d: Decimal) -> Decimal {
    item_base(q, p, d).expect("fits")
}

fn item(qty: &str, price: &str, discount: &str, rate: &str) -> LineData {
    LineData::Item(ItemData {
        description: "x".into(),
        quantity: dec(qty),
        unit: None,
        unit_price: dec(price),
        discount_pct: dec(discount),
        vat_rate: dec(rate),
    })
}

fn czk(mode: VatMode) -> Params {
    Params {
        vat_mode: mode,
        is_czk: true,
        exchange_rate: None,
        rounding: RoundingMode::None,
    }
}

fn eval(lines: &[LineData], p: Params) -> Totals {
    evaluate(lines, p).expect("valid").totals
}

#[test]
fn rounds_half_away_from_zero() {
    assert_eq!(ib(dec("1"), dec("0.005"), dec("0")), dec("0.01"));
    assert_eq!(ib(dec("1"), dec("-0.005"), dec("0")), dec("-0.01"));
    assert_eq!(ib(dec("1"), dec("0.015"), dec("0")), dec("0.02"));
    assert_eq!(ib(dec("3"), dec("0.0050"), dec("0")), dec("0.02"));
    assert_eq!(ib(dec("2"), dec("10"), dec("0")).to_string(), "20.00");
}

#[test]
fn applies_percentage_discount() {
    assert_eq!(ib(dec("2"), dec("100"), dec("10")), dec("180.00"));
    assert_eq!(ib(dec("1"), dec("99.99"), dec("33.33")), dec("66.66"));
    assert_eq!(ib(dec("1"), dec("100"), dec("100")), dec("0.00"));
}

#[test]
fn vat_is_computed_per_rate_not_per_line() {
    // Per line: 0.50 × 21 % = 0.105 → 0.11, three lines = 0.33.
    // Per rate (§37): 1.50 × 21 % = 0.315 → 0.32.
    let lines = vec![item("1", "0.5", "0", "21"); 3];
    let t = eval(&lines, czk(VatMode::Standard));
    assert_eq!(t.recap.len(), 1);
    assert_eq!(t.recap[0].base, dec("1.50"));
    assert_eq!(t.vat, dec("0.32"));
    assert_eq!(t.total, dec("1.82"));
}

#[test]
fn recap_groups_by_rate_sorted_desc() {
    let lines = [
        item("1", "100", "0", "12"),
        item("10", "100", "0", "21"),
        LineData::Text {
            description: "t".into(),
        },
        item("1", "50", "0", "0"),
        item("1", "100", "0", "21"),
    ];
    let t = eval(&lines, czk(VatMode::Standard));
    let rates: Vec<String> = t.recap.iter().map(|r| r.vat_rate.to_string()).collect();
    assert_eq!(rates, ["21", "12", "0"]);
    assert_eq!(t.recap[0].base, dec("1100"));
    assert_eq!(t.recap[0].vat, dec("231"));
    assert_eq!(t.recap[1].vat, dec("12"));
    assert_eq!(t.base, dec("1250"));
    assert_eq!(t.vat, dec("243"));
    assert_eq!(t.total, dec("1493"));
    assert_eq!(t.payable, dec("1493"));
    assert_eq!(t.rounding, Decimal::ZERO);
    assert_eq!(t.total_czk, None);
    assert_eq!(t.recap[0].base_czk, None);
}

#[test]
fn non_charging_modes_have_zero_vat() {
    for mode in [VatMode::ReverseCharge, VatMode::Exempt] {
        let t = eval(&[item("1", "100", "0", "21")], czk(mode));
        assert_eq!(t.recap[0].vat_rate, dec("21"), "{mode:?}");
        assert_eq!(t.vat, Decimal::ZERO, "{mode:?}");
        assert_eq!(t.total, dec("100"), "{mode:?}");
    }
    let t = eval(&[item("1", "100", "0", "0")], czk(VatMode::NonPayer));
    assert_eq!((t.vat, t.total), (Decimal::ZERO, dec("100")));
}

#[test]
fn non_payer_requires_zero_rates() {
    let lines = [item("1", "1", "0", "0"), item("1", "1", "0", "21")];
    let err = evaluate(&lines, czk(VatMode::NonPayer)).expect_err("invalid");
    let mut expected = FieldErrors::new();
    expected.add("lines.1.vatRate", "invalid");
    assert_eq!(err, expected);
}

#[test]
fn unit_mode_adds_a_rounding_line() {
    let p = Params {
        rounding: RoundingMode::Unit,
        ..czk(VatMode::Standard)
    };
    let t = eval(&[item("1", "100.40", "0", "21")], p);
    // 100.40 + 21.08 = 121.48 → 121
    assert_eq!(t.total, dec("121.48"));
    assert_eq!(t.payable, dec("121"));
    assert_eq!(t.rounding, dec("-0.48"));
    let t = eval(&[item("1", "100.50", "0", "0")], p);
    assert_eq!((t.payable, t.rounding), (dec("101"), dec("0.50")));
}

#[test]
fn half_mode_rounds_to_fifty_hundredths() {
    let p = Params {
        rounding: RoundingMode::Half,
        ..czk(VatMode::Standard)
    };
    // 100.40 + 21.08 = 121.48 → 121.50
    let t = eval(&[item("1", "100.40", "0", "21")], p);
    assert_eq!((t.payable, t.rounding), (dec("121.50"), dec("0.02")));
    // Half-way: 100.25 → 100.50; a credit note rounds symmetrically.
    let t = eval(&[item("1", "100.25", "0", "0")], p);
    assert_eq!((t.payable, t.rounding), (dec("100.50"), dec("0.25")));
    let t = eval(&[item("-1", "100.25", "0", "0")], p);
    assert_eq!((t.payable, t.rounding), (dec("-100.50"), dec("-0.25")));
}

#[test]
fn every_mode_works_for_a_foreign_currency() {
    let p = Params {
        vat_mode: VatMode::Standard,
        is_czk: false,
        exchange_rate: Some(dec("25")),
        rounding: RoundingMode::Half,
    };
    // 10.10 + 2.12 = 12.22 EUR → 12.00, CZK from the payable.
    let t = eval(&[item("1", "10.10", "0", "21")], p);
    assert_eq!((t.payable, t.rounding), (dec("12.00"), dec("-0.22")));
    assert_eq!(t.total_czk, Some(dec("300.00")));
    let unit = Params {
        rounding: RoundingMode::Unit,
        ..p
    };
    let t = eval(&[item("-1", "10.40", "0", "21")], unit);
    // -(10.40 + 2.18) = -12.58 → -13
    assert_eq!((t.payable, t.rounding), (dec("-13.00"), dec("-0.42")));
}

#[test]
fn foreign_currency_converts_to_czk() {
    let p = Params {
        vat_mode: VatMode::Standard,
        is_czk: false,
        exchange_rate: Some(dec("25.125")),
        rounding: RoundingMode::None,
    };
    let t = eval(&[item("1", "100", "0", "21")], p);
    assert_eq!(t.recap[0].base_czk, Some(dec("2512.50")));
    // 21 × 25.125 = 527.625 → 527.63
    assert_eq!(t.recap[0].vat_czk, Some(dec("527.63")));
    // 121 × 25.125 = 3040.125 → 3040.13
    assert_eq!(t.total_czk, Some(dec("3040.13")));
    let unknown = Params {
        exchange_rate: None,
        ..p
    };
    assert_eq!(
        eval(&[item("1", "100", "0", "21")], unknown).total_czk,
        None
    );
}

#[test]
fn subtotals_are_display_only() {
    let lines = [
        item("1", "100", "0", "21"),
        item("2", "50", "0", "21"),
        LineData::Subtotal {
            description: "s".into(),
            refs: vec![1, 2],
            collapse: true,
        },
    ];
    let ev = evaluate(&lines, czk(VatMode::Standard)).expect("valid");
    assert_eq!(ev.lines[2], Some((dec("21"), dec("200"))));
    assert_eq!(ev.totals.base, dec("200"));
    let bad = [
        item("1", "1", "0", "21"),
        item("1", "1", "0", "12"),
        lines[2].clone(),
    ];
    let err = evaluate(&bad, czk(VatMode::Standard)).expect_err("mixed");
    let mut expected = FieldErrors::new();
    expected.add("lines.2.refs", "invalid");
    assert_eq!(err, expected);
}

#[test]
fn rejects_totals_that_do_not_fit_storage() {
    let err = evaluate(
        &[item("999999999", "999999999", "0", "0")],
        czk(VatMode::Standard),
    )
    .expect_err("too big");
    let mut expected = FieldErrors::new();
    expected.add("lines", "invalid");
    assert_eq!(err, expected);
}

fn eur(rate: &str) -> Params {
    Params {
        vat_mode: VatMode::Standard,
        is_czk: false,
        exchange_rate: Some(dec(rate)),
        rounding: RoundingMode::None,
    }
}

fn only(field: &str) -> FieldErrors {
    let mut e = FieldErrors::new();
    e.add(field, "invalid");
    e
}

#[test]
fn overflow_is_a_validation_error_not_a_panic() {
    // Review case: 10^9 × 10^9 = 10^18 base, × an 11-digit rate exceeds Decimal.
    let huge = [item("1000000000", "1000000000", "0", "21")];
    assert_eq!(
        evaluate(&huge, eur("99999999999")).expect_err("e"),
        only("lines")
    );
    assert_eq!(
        item_base(dec("999999999999.9999"), dec("999999999999.9999"), dec("0")),
        None
    );
    // Many lines that each fit but sum beyond the column.
    let many = vec![item("1000000", "999999999", "0", "21"); 20];
    assert_eq!(
        evaluate(&many, czk(VatMode::Standard)).expect_err("e"),
        only("lines")
    );
    // Amounts fit, only the CZK conversion does not.
    let fine = [item("1000000", "1000000", "0", "21")];
    assert_eq!(
        evaluate(&fine, eur("99999999999")).expect_err("e"),
        only("exchangeRate")
    );
    assert!(evaluate(&fine, eur("25")).is_ok());
}

#[test]
fn exploding_nested_subtotals_are_rejected() {
    // Each subtotal sums the previous two: Fibonacci growth overflows Decimal.
    let mut lines = vec![
        item("1", "999999999", "0", "21"),
        item("1", "999999999", "0", "21"),
    ];
    for i in 2..200 {
        lines.push(LineData::Subtotal {
            description: String::new(),
            refs: vec![i - 1, i],
            collapse: false,
        });
    }
    let err = evaluate(&lines, czk(VatMode::Standard)).expect_err("overflow");
    assert!(!err.is_empty());
}
