use super::*;
use crate::overview::period::chart;

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

fn date(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).expect("date")
}

fn doc(
    direction: Direction,
    doc_type: DocType,
    tax: NaiveDate,
    recap: &[(&str, &str, &str)],
) -> Doc {
    let recap: Vec<_> = recap.iter().map(|(r, b, v)| (d(r), d(b), d(v))).collect();
    let total = recap.iter().map(|r| r.1 + r.2).sum();
    Doc {
        direction,
        doc_type,
        vat_mode: VatMode::Standard,
        vat_deductible: true,
        tax_date: tax,
        category_id: None,
        recap,
        total,
    }
}

fn issued(tax: NaiveDate, recap: &[(&str, &str, &str)]) -> Doc {
    doc(Direction::Issued, DocType::Invoice, tax, recap)
}

fn received(tax: NaiveDate, recap: &[(&str, &str, &str)]) -> Doc {
    doc(Direction::Received, DocType::Invoice, tax, recap)
}

fn january(docs: &[Doc], vat_payer: bool) -> Figures {
    let (from, to) = (date(1, 1), date(1, 31));
    let c = chart(from, to).expect("chart");
    figures(docs, from, to, &c, vat_payer, &HashMap::new())
}

#[test]
fn payer_totals_use_the_base_and_credit_notes_reduce() {
    let mut rounded = issued(date(1, 10), &[("21", "1000", "210")]);
    rounded.total = d("1210.40");
    let docs = [
        rounded,
        issued(date(1, 20), &[("12", "100", "12")]),
        doc(
            Direction::Issued,
            DocType::CreditNote,
            date(1, 25),
            &[("21", "200", "42")],
        ),
        received(date(1, 5), &[("21", "500", "105")]),
        // Outside the period: only in the chart.
        issued(date(2, 1), &[("21", "9999", "0")]),
    ];
    let f = january(&docs, true);
    assert_eq!(f.income.count, 3);
    assert_eq!(f.income.amount, d("900"));
    assert_eq!((f.income.base, f.income.vat), (d("900"), d("180")));
    assert_eq!(f.income.total, d("1080.40"), "rounding in the total only");
    assert_eq!((f.expense.count, f.expense.amount), (1, d("500")));

    let n = january(&docs, false);
    assert_eq!(n.income.amount, d("1080.40"), "non-payer: totals");
    assert_eq!(n.expense.amount, d("605"));
}

#[test]
fn vat_per_rate_reverse_charge_and_non_deductible() {
    let mut rc_out = issued(date(1, 3), &[("0", "700", "0")]);
    rc_out.vat_mode = VatMode::ReverseCharge;
    let mut rc_in = received(date(1, 3), &[("21", "300", "0")]);
    rc_in.vat_mode = VatMode::ReverseCharge;
    let mut lunch = received(date(1, 4), &[("12", "100", "12")]);
    lunch.vat_deductible = false;
    let mut exempt = issued(date(1, 4), &[("0", "50", "0")]);
    exempt.vat_mode = VatMode::Exempt;
    let docs = [
        issued(date(1, 2), &[("21", "1000", "210"), ("12.00", "100", "12")]),
        issued(date(1, 9), &[("21", "100", "21")]),
        doc(
            Direction::Received,
            DocType::CreditNote,
            date(1, 9),
            &[("21", "100", "21")],
        ),
        received(date(1, 9), &[("21", "400", "84")]),
        rc_out,
        rc_in,
        lunch,
        exempt,
    ];
    let f = january(&docs, true);
    let v = &f.vat;
    let rows = |r: &[VatRow]| {
        r.iter()
            .map(|r| (r.rate, r.base, r.vat))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rows(&v.output),
        [(d("21"), d("1100"), d("231")), (d("12"), d("100"), d("12"))]
    );
    assert_eq!(rows(&v.input), [(d("21"), d("300"), d("63"))]);
    assert_eq!(
        (v.output_vat, v.input_vat, v.result),
        (d("243"), d("63"), d("180"))
    );
    assert_eq!(
        (v.reverse_charge_issued, v.reverse_charge_received),
        (d("700"), d("300"))
    );
    assert_eq!(
        (v.non_deductible.base, v.non_deductible.vat),
        (d("100"), d("12"))
    );
    // The non-deductible VAT is part of the expense.
    assert_eq!(f.expense.amount, d("300") + d("300") + d("112"));

    let refund = january(&[received(date(1, 9), &[("21", "100", "21")])], true);
    assert_eq!(refund.vat.result, d("-21"));
}

#[test]
fn months_cover_the_chart_and_mark_the_period() {
    let docs = [
        issued(date(1, 31), &[("21", "100", "21")]),
        received(date(1, 1), &[("21", "40", "0")]),
        issued(date(1, 15), &[("21", "1", "0")]),
    ];
    let mut older = issued(date(1, 1), &[("21", "7", "0")]);
    older.tax_date = NaiveDate::from_ymd_opt(2025, 3, 2).expect("date");
    let mut too_old = older.clone();
    too_old.tax_date = NaiveDate::from_ymd_opt(2025, 1, 31).expect("date");
    let all: Vec<_> = docs.into_iter().chain([older, too_old]).collect();
    let f = january(&all, true);
    assert_eq!(f.months.len(), 12);
    let jan = f.months.last().expect("january");
    assert_eq!(
        (jan.month.as_str(), jan.income, jan.expense, jan.in_period),
        ("2026-01", d("101"), d("40"), true)
    );
    let march = &f.months[1];
    assert_eq!(
        (march.month.as_str(), march.income, march.in_period),
        ("2025-03", d("7"), false)
    );
    assert!(f.months[..11].iter().all(|m| !m.in_period));
    assert_eq!(
        f.income.count, 2,
        "chart-only documents stay out of the period"
    );
}

#[test]
fn categories_group_with_an_uncategorized_bucket() {
    let (soft, rent) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let names = HashMap::from([(soft, "Software".to_string()), (rent, "Nájem".to_string())]);
    let with = |c: Option<Uuid>, base: &str| {
        let mut r = received(date(1, 2), &[("21", base, "0")]);
        r.category_id = c;
        r
    };
    let docs = [
        with(Some(soft), "100"),
        with(Some(rent), "500"),
        with(Some(soft), "50"),
        with(None, "150"),
        issued(date(1, 2), &[("21", "10", "0")]),
    ];
    let c = chart(date(1, 1), date(1, 31)).expect("chart");
    let f = figures(&docs, date(1, 1), date(1, 31), &c, true, &names);
    let got: Vec<_> = f
        .categories
        .expense
        .iter()
        .map(|c| (c.name.as_deref(), c.count, c.amount))
        .collect();
    assert_eq!(
        got,
        [
            (Some("Nájem"), 1, d("500")),
            (Some("Software"), 2, d("150")),
            (None, 1, d("150")),
        ]
    );
    assert_eq!(f.categories.income.len(), 1);
    assert_eq!(f.categories.income[0].id, None);
}
