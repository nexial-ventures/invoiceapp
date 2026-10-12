use super::*;
use crate::document::compute::item_base;
use crate::document::line::{Direction, ItemData};
use crate::isdoc::parse::parse;
use crate::isdoc::plan::plan;

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

fn day(s: &str) -> NaiveDate {
    s.parse().expect("date")
}

fn party(name: &str, ico: &str, vat_payer: Option<bool>) -> PartySnapshot {
    PartySnapshot {
        name: name.into(),
        ico: Some(ico.into()),
        dic: Some(format!("CZ{ico}")),
        street: "Ulice 1".into(),
        city: "Praha".into(),
        zip: "11000".into(),
        country: "CZ".into(),
        registration: Some("C 1 vedená u MS v Praze".into()),
        vat_payer,
        email: Some("a@example.com".into()),
        phone: None,
        web: None,
    }
}

fn item(desc: &str, qty: &str, price: &str, rate: &str) -> LineData {
    LineData::Item(ItemData {
        description: desc.into(),
        quantity: d(qty),
        unit: Some("h".into()),
        unit_price: d(price),
        discount_pct: Decimal::ZERO,
        vat_rate: d(rate),
    })
}

fn row(rate: &str, base: &str, vat: &str) -> RecapRow {
    RecapRow {
        vat_rate: d(rate),
        base: d(base),
        vat: d(vat),
        base_czk: None,
        vat_czk: None,
    }
}

fn invoice() -> Source {
    Source {
        id: Uuid::from_u128(42),
        doc_type: DocType::Invoice,
        number: "20260007".into(),
        issue_date: day("2026-10-01"),
        tax_point_date: Some(day("2026-09-30")),
        due_date: Some(day("2026-10-15")),
        currency: "CZK".into(),
        rate: None,
        vat_mode: VatMode::Standard,
        supplier: party("Dodavatel s.r.o.", "44444443", Some(true)),
        customer: Some(party("Odběratel a.s.", "12345679", None)),
        bank: Some(BankSnapshot {
            account_number: Some("19-2000145399/0800".into()),
            iban: Some("CZ6508000000192000145399".into()),
            bic: Some("GIBACZPX".into()),
        }),
        payment_method: PaymentMethod::BankTransfer,
        variable_symbol: Some("20260007".into()),
        constant_symbol: None,
        note: Some("Poznámka & <x>".into()),
        lines: vec![
            item("Práce", "10", "100", "21"),
            LineData::Text {
                description: "Text".into(),
            },
            item("Kniha", "1", "50", "12"),
        ],
        recap: vec![row("21", "1000.00", "210.00"), row("12", "50.00", "6.00")],
        rounding: d("-0.00"),
        payable: d("1266.00"),
        total_czk: None,
        original: None,
        deposits: Vec::new(),
        supplement: None,
    }
}

#[test]
fn round_trip_through_the_parser() {
    let xml = render(&invoice());
    let parsed = parse(xml.as_bytes()).expect("parsed");
    let p = plan(parsed, Some("44444443")).expect("plan");
    assert_eq!(p.direction, Direction::Issued);
    assert_eq!(p.number, "20260007");
    assert_eq!(p.tax_point_date, Some(day("2026-09-30")));
    assert_eq!(p.due_date, Some(day("2026-10-15")));
    assert_eq!(p.vat_mode, VatMode::Standard);
    assert_eq!(p.totals.recap, invoice().recap);
    assert_eq!((p.totals.base, p.totals.vat), (d("1050.00"), d("216.00")));
    assert_eq!(p.totals.payable, d("1266.00"));
    assert_eq!(p.lines, invoice().lines);
    assert_eq!(p.note.as_deref(), Some("Poznámka & <x>"));
    assert_eq!(p.customer.and_then(|c| c.ico).as_deref(), Some("12345679"));
    assert_eq!(
        p.supplier.registration.as_deref(),
        Some("C 1 vedená u MS v Praze")
    );
    let bank = p.bank.expect("bank");
    assert_eq!(bank.account_number.as_deref(), Some("19-2000145399/0800"));
    assert_eq!(bank.bic.as_deref(), Some("GIBACZPX"));
}

#[test]
fn credit_note_is_positive_with_its_original() {
    let mut s = invoice();
    s.doc_type = DocType::CreditNote;
    s.original = Some(("20260001".into(), day("2026-09-01")));
    let xml = render(&s);
    assert!(xml.contains("<DocumentType>2</DocumentType>"));
    assert!(xml.contains("<PayableAmount>1266.00</PayableAmount>"));
    assert!(!xml.contains(">-"), "no negative amounts");
    let p = plan(parse(xml.as_bytes()).expect("parsed"), Some("44444443")).expect("plan");
    assert_eq!(p.original_ref.as_deref(), Some("20260001"));
    assert_eq!(p.totals.payable, d("1266.00"));
}

#[test]
fn foreign_currency_carries_both_amounts() {
    let mut s = invoice();
    s.currency = "EUR".into();
    s.rate = Some(d("24.5"));
    s.recap = vec![RecapRow {
        base_czk: Some(d("24500.00")),
        vat_czk: Some(d("5145.00")),
        ..row("21", "1000.00", "210.00")
    }];
    s.lines = vec![item("Práce", "10", "100", "21")];
    s.payable = d("1210.00");
    s.total_czk = Some(d("29645.00"));
    let xml = render(&s);
    assert!(
        xml.contains("<ForeignCurrencyCode>EUR</ForeignCurrencyCode><CurrRate>24.5</CurrRate>")
    );
    assert!(xml.contains(
        "<PayableAmount>29645.00</PayableAmount><PayableAmountCurr>1210.00</PayableAmountCurr>"
    ));
    let p = plan(parse(xml.as_bytes()).expect("parsed"), Some("44444443")).expect("plan");
    assert_eq!((p.currency.as_str(), p.rate), ("EUR", Some(d("24.5"))));
    assert_eq!(p.totals.recap, s.recap);
    assert_eq!(p.totals.total_czk, Some(d("29645.00")));
    assert_eq!(p.lines, s.lines);
}

#[test]
fn modes_survive_the_round_trip() {
    for (mode, rate) in [
        (VatMode::NonPayer, "0"),
        (VatMode::ReverseCharge, "21"),
        (VatMode::Exempt, "21"),
    ] {
        let mut s = invoice();
        s.vat_mode = mode;
        s.lines = vec![item("Práce", "10", "100", rate)];
        s.recap = vec![row(rate, "1000.00", "0.00")];
        s.payable = d("1000.00");
        let p = plan(
            parse(render(&s).as_bytes()).expect("parsed"),
            Some("44444443"),
        )
        .expect("plan");
        assert_eq!(p.vat_mode, mode);
    }
}

#[test]
fn taxed_deposit_is_already_claimed() {
    let mut s = invoice();
    s.lines
        .push(LineData::Advance(crate::document::line::AdvanceData {
            document_id: Uuid::from_u128(7),
            description: "Odpočet".into(),
            recap: vec![AdvanceRow {
                vat_rate: d("21"),
                base: d("400.00"),
                vat: d("84.00"),
                base_czk: None,
                vat_czk: None,
            }],
        }));
    s.recap = vec![row("21", "600.00", "126.00"), row("12", "50.00", "6.00")];
    s.payable = d("782.00");
    s.deposits = vec![Deposit {
        number: "DP20260001".into(),
        variable_symbol: "20260001".into(),
        taxed: true,
        rows: vec![AdvanceRow {
            vat_rate: d("21"),
            base: d("400.00"),
            vat: d("84.00"),
            base_czk: None,
            vat_czk: None,
        }],
    }];
    let xml = render(&s);
    assert!(xml.contains("<TaxedDeposits><TaxedDeposit><ID>DP20260001</ID>"));
    assert!(xml.contains("<TaxableAmount>1000.00</TaxableAmount>"));
    assert!(xml.contains("<AlreadyClaimedTaxableAmount>400.00</AlreadyClaimedTaxableAmount>"));
    assert!(xml.contains("<DifferenceTaxableAmount>600.00</DifferenceTaxableAmount>"));
    let p = plan(parse(xml.as_bytes()).expect("parsed"), Some("44444443")).expect("plan");
    assert_eq!(p.totals.recap, s.recap, "the stored (net) recap comes back");
    assert_eq!(p.totals.payable, d("782.00"));
    assert_eq!(
        p.lines.len(),
        3,
        "the advance line is a deposit, not a line"
    );
}

#[test]
fn anonymous_customer_and_supplement() {
    let mut s = invoice();
    s.doc_type = DocType::Simplified;
    s.customer = None;
    s.supplement = Some(Supplement {
        filename: "20260007.pdf".into(),
        sha256: vec![0; 32],
    });
    let xml = render(&s);
    assert!(xml.contains("<AnonymousCustomerParty>"));
    assert!(xml.contains("<Supplement preview=\"true\"><Filename>20260007.pdf</Filename>"));
    let p = parse(xml.as_bytes()).expect("parsed");
    assert_eq!(p.customer, None);
    assert_eq!(
        crate::isdoc::parse::preview_file_of(xml.as_bytes()).as_deref(),
        Some("20260007.pdf")
    );
    assert!(manifest("a.isdoc").contains("<maindocument filename=\"a.isdoc\"></maindocument>"));
}

#[test]
fn non_taxed_deposit_round_trips() {
    // A non-payer final invoice deducting a paid proforma (no DDPP).
    let mut s = invoice();
    s.vat_mode = VatMode::NonPayer;
    s.supplier.vat_payer = Some(false);
    let row0 = AdvanceRow {
        vat_rate: Decimal::ZERO,
        base: d("400.00"),
        vat: Decimal::ZERO,
        base_czk: None,
        vat_czk: None,
    };
    s.lines = vec![
        item("Práce", "10", "100", "0"),
        LineData::Advance(crate::document::line::AdvanceData {
            document_id: Uuid::from_u128(9),
            description: "Odpočet".into(),
            recap: vec![row0.clone()],
        }),
    ];
    s.recap = vec![row("0", "600.00", "0.00")];
    s.payable = d("600.00");
    s.deposits = vec![Deposit {
        number: "Z20260001".into(),
        variable_symbol: "20260001".into(),
        taxed: false,
        rows: vec![row0],
    }];
    let xml = render(&s);
    assert!(xml.contains("<NonTaxedDeposits><NonTaxedDeposit><ID>Z20260001</ID>"));
    assert!(xml.contains("<PaidDepositsAmount>400.00</PaidDepositsAmount>"));
    assert!(xml.contains("<DifferenceTaxInclusiveAmount>1000.00</DifferenceTaxInclusiveAmount>"));
    let p = plan(parse(xml.as_bytes()).expect("parsed"), Some("44444443")).expect("plan");
    assert_eq!(p.vat_mode, VatMode::NonPayer);
    assert_eq!(p.totals.recap, s.recap);
    assert_eq!(
        (p.totals.total, p.totals.payable),
        (d("600.00"), d("600.00"))
    );
    assert_eq!(p.totals.payable, p.totals.total + p.totals.rounding);
}

#[test]
fn foreign_discount_keeps_the_base_only() {
    // ISDOC 6.0.2 has no document-currency unit price or before-discount
    // amount (`UnitPrice`, `LineExtensionAmountBeforeDiscount` are CZK only),
    // so a discounted foreign line comes back at its net unit price.
    let mut s = invoice();
    s.currency = "EUR".into();
    s.rate = Some(d("24.5"));
    s.lines = vec![LineData::Item(ItemData {
        description: "Licence".into(),
        quantity: d("1"),
        unit: None,
        unit_price: d("299.99"),
        discount_pct: d("15"),
        vat_rate: d("21"),
    })];
    s.recap = vec![RecapRow {
        base_czk: Some(d("6247.26")),
        vat_czk: Some(d("1311.92")),
        ..row("21", "254.99", "53.55")
    }];
    s.payable = d("308.54");
    s.total_czk = Some(d("7559.23"));
    let xml = render(&s);
    assert!(!xml.contains("BeforeDiscount"));
    let p = plan(parse(xml.as_bytes()).expect("parsed"), Some("44444443")).expect("plan");
    let LineData::Item(i) = &p.lines[0] else {
        panic!("item expected");
    };
    assert_eq!((i.unit_price, i.discount_pct), (d("254.99"), d("0")));
    assert_eq!(
        item_base(i.quantity, i.unit_price, i.discount_pct),
        Some(d("254.99")),
        "the line base survives"
    );
    assert_eq!(p.totals.recap, s.recap);
}
