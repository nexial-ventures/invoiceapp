use super::*;
use crate::csvio::cell::{INVALID_DATE, Row};

const HEADER: &str = "direction;doc_type;number;supplier_number;related_number;issue_date;tax_date;\
due_date;received_date;counterparty_name;counterparty_ico;counterparty_dic;counterparty_country;\
currency;exchange_rate;vat_mode;base_21;vat_21;base_12;vat_12;base_0;rounding;total;paid_date;\
variable_symbol;vat_deductible;category;note";

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

fn ctx_payer(vat_payer: bool) -> Ctx {
    Ctx { vat_payer, ..ctx() }
}

fn ctx() -> Ctx {
    Ctx {
        company: Party {
            name: "Dodavatel s.r.o.".into(),
            ico: Some("44444443".into()),
            ..Default::default()
        },
        vat_payer: true,
        locale: "cs".into(),
    }
}

/// A row from `name=value` pairs over [`HEADER`].
fn run_with(pairs: &[(&str, &str)], ctx: &Ctx) -> Check<Mapped> {
    let header: Vec<String> = HEADER.split(';').map(str::to_string).collect();
    let cols = Columns::new(&header).expect("columns");
    let cells: Vec<Cell> = header
        .iter()
        .map(|h| match pairs.iter().find(|(k, _)| k == h) {
            Some((_, v)) if !v.is_empty() => Cell::Text(v.to_string()),
            _ => Cell::Empty,
        })
        .collect();
    map(
        Row {
            cols: &cols,
            cells: &cells,
        },
        ctx,
    )
}

fn run(pairs: &[(&str, &str)]) -> Check<Mapped> {
    run_with(pairs, &ctx())
}

fn issued(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut v = vec![
        ("direction", "issued"),
        ("doc_type", "invoice"),
        ("number", "2026001"),
        ("issue_date", "15.01.2026"),
        ("counterparty_name", "Fiktivní Odběratel s.r.o."),
        ("counterparty_ico", "12345679"),
        ("base_21", "1000,00"),
        ("vat_21", "210,00"),
        ("total", "1210,00"),
    ];
    for (k, val) in extra {
        v.retain(|(kk, _)| kk != k);
        v.push((k, val));
    }
    v
}

fn received(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut v = issued(&[("direction", "received"), ("supplier_number", "FV-1")]);
    v.retain(|(k, _)| *k != "number");
    for (k, val) in extra {
        v.retain(|(kk, _)| kk != k);
        v.push((k, val));
    }
    v
}

fn fails(pairs: &[(&str, &str)], code: &str, field: Option<&str>) {
    let e = run(pairs).expect_err("row error");
    assert_eq!((e.code, e.field.as_deref()), (code, field), "{pairs:?}");
}

#[test]
fn issued_czk_invoice() {
    let m = run(&issued(&[
        ("paid_date", "2026-01-20"),
        ("category", "Služby"),
    ]))
    .expect("row");
    let p = &m.plan;
    assert_eq!(
        (p.direction, p.doc_type, p.number.as_str()),
        (Direction::Issued, DocType::Invoice, "2026001")
    );
    assert_eq!(p.tax_point_date, p.due_date);
    assert_eq!(p.tax_point_date, Some(p.issue_date));
    assert_eq!(p.received_date, None);
    assert_eq!(p.supplier.ico.as_deref(), Some("44444443"));
    assert_eq!(p.vat_payer, Some(true));
    assert_eq!(p.customer.as_ref().map(|c| c.country.as_str()), Some("CZ"));
    assert_eq!(
        (p.totals.total, p.totals.payable, p.gross),
        (d("1210"), d("1210"), d("1210"))
    );
    assert_eq!(p.totals.recap.len(), 1);
    assert_eq!(p.payment_method, PaymentMethod::BankTransfer);
    assert_eq!(m.category.as_deref(), Some("Služby"));
    assert!(m.paid_date.is_some());
    assert!(!m.vat_deductible, "issued rows never deduct");
}

#[test]
fn received_defaults() {
    let m = run(&received(&[("tax_date", "20.1.2026")])).expect("row");
    let p = &m.plan;
    assert_eq!(p.number, "FV-1");
    assert_eq!(p.received_date, p.tax_point_date);
    assert_eq!(p.supplier.ico.as_deref(), Some("12345679"));
    assert_eq!((p.customer.as_ref(), p.vat_payer), (None, None));
    assert!(m.vat_deductible);
    let m = run(&received(&[
        ("vat_deductible", "ne"),
        ("received_date", "2026-02-01"),
    ]))
    .expect("row");
    assert!(!m.vat_deductible);
    assert_eq!(
        m.plan.received_date.map(|d| d.to_string()).as_deref(),
        Some("2026-02-01")
    );
    // A received DDPP may have no due date; an issued one gets the issue date.
    let ddpp = run(&received(&[("doc_type", "advance_tax_doc")])).expect("row");
    assert_eq!(ddpp.plan.due_date, None);
    let ddpp = run(&issued(&[("doc_type", "advance_tax_doc")])).expect("row");
    assert!(ddpp.plan.due_date.is_some());
    // Reverse charge: no VAT, not deductible by default.
    let rc = run(&received(&[
        ("vat_mode", "reverse_charge"),
        ("vat_21", ""),
        ("total", "1000"),
    ]));
    assert!(!rc.expect("row").vat_deductible);
}

#[test]
fn credit_notes_are_negated_as_a_whole() {
    let m = run(&issued(&[
        ("doc_type", "credit_note"),
        ("base_21", "-1000"),
        ("vat_21", "-210"),
        ("rounding", "0,40"),
        ("total", "-1209,60"),
    ]))
    .expect("row");
    let t = &m.plan.totals;
    assert_eq!((t.recap[0].base, t.recap[0].vat), (d("1000"), d("210")));
    assert_eq!((t.rounding, t.payable), (d("-0.40"), d("1209.60")));
    // Written positive: taken as is.
    let m = run(&issued(&[("doc_type", "advance_credit_note")])).expect("row");
    assert_eq!(m.plan.totals.payable, d("1210"));
    // Mixed signs.
    fails(
        &issued(&[
            ("doc_type", "credit_note"),
            ("base_21", "-1000"),
            ("vat_21", "210"),
            ("total", "-790"),
        ]),
        INVALID_AMOUNT,
        Some("vat_21"),
    );
    // Any other type must be ≥ 0.
    fails(
        &issued(&[("base_21", "-1000"), ("vat_21", "-210"), ("total", "-1210")]),
        INVALID_AMOUNT,
        Some("base_21"),
    );
    let m =
        run(&issued(&[("rounding", "-0,40"), ("total", "1209,60")])).expect("negative rounding");
    assert_eq!(m.plan.totals.rounding, d("-0.40"));
}

#[test]
fn foreign_currency() {
    let m = run(&received(&[
        ("currency", "eur"),
        ("exchange_rate", "24,335"),
        ("base_21", "2433,50"),
        ("vat_21", "511,04"),
        ("total", "121,00"),
    ]))
    .expect("row");
    let p = &m.plan;
    assert_eq!((p.currency.as_str(), p.rate), ("EUR", Some(d("24.335"))));
    assert_eq!(p.totals.recap[0].base, d("100"));
    assert_eq!(p.totals.total_czk, Some(d("2944.54")));
    fails(
        &received(&[("currency", "EUR")]),
        MISSING_FIELD,
        Some("exchange_rate"),
    );
    fails(
        &received(&[("currency", "EUR"), ("exchange_rate", "0")]),
        INVALID_AMOUNT,
        Some("exchange_rate"),
    );
    fails(
        &received(&[("currency", "EUR"), ("exchange_rate", "1,1234567")]),
        INVALID_AMOUNT,
        Some("exchange_rate"),
    );
    fails(
        &issued(&[("exchange_rate", "25")]),
        INVALID_VALUE,
        Some("exchange_rate"),
    );
    assert!(run(&issued(&[("exchange_rate", "1")])).is_ok());
    fails(
        &issued(&[("currency", "EURO")]),
        INVALID_VALUE,
        Some("currency"),
    );
}

#[test]
fn row_errors() {
    fails(
        &issued(&[("direction", "")]),
        MISSING_FIELD,
        Some("direction"),
    );
    fails(
        &issued(&[("direction", "sent")]),
        INVALID_VALUE,
        Some("direction"),
    );
    fails(
        &issued(&[("doc_type", "order")]),
        INVALID_VALUE,
        Some("doc_type"),
    );
    fails(&issued(&[("number", "")]), MISSING_FIELD, Some("number"));
    fails(
        &received(&[("supplier_number", "")]),
        MISSING_FIELD,
        Some("supplier_number"),
    );
    fails(
        &issued(&[("issue_date", "32.1.2026")]),
        INVALID_DATE,
        Some("issue_date"),
    );
    fails(
        &issued(&[("counterparty_name", "")]),
        MISSING_FIELD,
        Some("counterparty_name"),
    );
    fails(
        &issued(&[("counterparty_ico", "12345678")]),
        INVALID_VALUE,
        Some("counterparty_ico"),
    );
    fails(
        &issued(&[("counterparty_ico", "44444443")]),
        INVALID_VALUE,
        Some("counterparty_ico"),
    );
    fails(
        &issued(&[("counterparty_dic", "12")]),
        INVALID_VALUE,
        Some("counterparty_dic"),
    );
    fails(
        &issued(&[("counterparty_country", "CZE")]),
        INVALID_VALUE,
        Some("counterparty_country"),
    );
    fails(
        &issued(&[("vat_mode", "zero")]),
        INVALID_VALUE,
        Some("vat_mode"),
    );
    fails(
        &issued(&[("variable_symbol", "12a")]),
        INVALID_VALUE,
        Some("variable_symbol"),
    );
    fails(
        &issued(&[("variable_symbol", "12345678901")]),
        INVALID_VALUE,
        Some("variable_symbol"),
    );
    fails(
        &received(&[("vat_deductible", "maybe")]),
        INVALID_VALUE,
        Some("vat_deductible"),
    );
    fails(&issued(&[("total", "")]), MISSING_FIELD, Some("total"));
    fails(
        &issued(&[("total", "1210,001")]),
        INVALID_AMOUNT,
        Some("total"),
    );
    fails(&issued(&[("total", "1211")]), TOTAL_MISMATCH, Some("total"));
    fails(
        &issued(&[("rounding", "100"), ("total", "1310")]),
        INVALID_AMOUNT,
        Some("rounding"),
    );
    fails(
        &issued(&[("base_21", ""), ("vat_21", ""), ("total", "0")]),
        NO_VAT_ROWS,
        None,
    );
    fails(
        &issued(&[("doc_type", "proforma"), ("tax_date", "1.1.2026")]),
        NOT_ALLOWED,
        Some("tax_date"),
    );
    fails(
        &received(&[("doc_type", "simplified"), ("counterparty_name", "")]),
        NOT_ALLOWED,
        Some("counterparty_name"),
    );
    fails(
        &issued(&[("vat_mode", "exempt")]),
        NOT_ALLOWED,
        Some("vat_21"),
    );
    fails(
        &issued(&[("doc_type", "advance_tax_doc"), ("paid_date", "1.2.2026")]),
        NOT_ALLOWED,
        Some("paid_date"),
    );
    let long = "x".repeat(41);
    let long: &'static str = Box::leak(long.into_boxed_str());
    fails(&issued(&[("number", long)]), INVALID_VALUE, Some("number"));
}

#[test]
fn simplified_without_customer_and_proforma() {
    let m = run(&issued(&[
        ("doc_type", "simplified"),
        ("counterparty_name", ""),
    ]))
    .expect("row");
    assert_eq!(m.plan.customer, None);
    assert_eq!(m.plan.counterparty(), None);
    let m = run(&issued(&[("doc_type", "proforma")])).expect("row");
    assert_eq!(m.plan.tax_point_date, None);
}

#[test]
fn zero_rate_and_several_rates() {
    let m = run(&issued(&[
        ("base_12", "100"),
        ("vat_12", "12"),
        ("base_0", "50"),
        ("total", "1372"),
        ("related_number", "2025099"),
    ]))
    .expect("row");
    let rates: Vec<String> = m
        .plan
        .totals
        .recap
        .iter()
        .map(|r| r.vat_rate.to_string())
        .collect();
    assert_eq!(rates, ["21", "12", "0"]);
    assert_eq!(m.plan.original_ref.as_deref(), Some("2025099"));
}

#[test]
fn non_payer_company_defaults_issued_rows_to_non_payer() {
    let mapped = |pairs: &[(&str, &str)]| run_with(pairs, &ctx_payer(false));
    let no_vat = issued(&[("vat_21", ""), ("total", "1000")]);
    let m = mapped(&no_vat).expect("row");
    assert_eq!(m.plan.vat_mode, VatMode::NonPayer);
    assert_eq!(m.plan.vat_payer, Some(false));
    let e = mapped(&issued(&[])).expect_err("VAT of a non-payer");
    assert_eq!((e.code, e.field.as_deref()), (NOT_ALLOWED, Some("vat_21")));
    // An explicit mode wins; received rows keep `standard`.
    let m = mapped(&issued(&[("vat_mode", "standard")])).expect("row");
    assert_eq!(m.plan.vat_mode, VatMode::Standard);
    let m = mapped(&received(&[])).expect("row");
    assert_eq!(m.plan.vat_mode, VatMode::Standard);
    // A payer company: `standard`.
    assert_eq!(run(&no_vat).expect("row").plan.vat_mode, VatMode::Standard);
}
