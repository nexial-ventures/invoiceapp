use super::*;
use crate::isdoc::parse::parse;
use crate::isdoc::parse::tests::{CREDIT, EUR, NONPAYER, VAT};

const US: Option<&str> = Some("44444443");

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

fn plan_of(xml: &str) -> Result<Plan, Code> {
    plan(parse(xml.as_bytes()).expect("parsed"), US)
}

#[test]
fn direction_by_company_ico() {
    assert_eq!(
        plan_of(NONPAYER).expect("plan").direction,
        Direction::Issued
    );
    let r = plan_of(VAT).expect("plan");
    assert_eq!(r.direction, Direction::Received);
    assert_eq!(
        r.counterparty().and_then(|p| p.ico.as_deref()),
        Some("87654326")
    );
    let spaced = plan(parse(VAT.as_bytes()).expect("parsed"), Some(" 4444 4443 "));
    assert_eq!(spaced.expect("plan").direction, Direction::Received);
    assert_eq!(
        plan(parse(VAT.as_bytes()).expect("parsed"), Some("11111111")),
        Err(FOREIGN)
    );
    assert_eq!(
        plan(parse(VAT.as_bytes()).expect("parsed"), None),
        Err(FOREIGN)
    );
    let both = VAT.replace("8765 4326", "44444443");
    assert_eq!(plan_of(&both), Err(AMBIGUOUS));
}

#[test]
fn non_payer_issued_invoice() {
    let p = plan_of(NONPAYER).expect("plan");
    assert_eq!(p.vat_mode, VatMode::NonPayer);
    assert_eq!(
        p.tax_point_date,
        Some(p.issue_date),
        "falls back to issue date"
    );
    assert_eq!(p.due_date, Some("2026-10-16".parse().expect("date")));
    assert_eq!(p.payment_method, PaymentMethod::BankTransfer);
    assert_eq!(p.variable_symbol.as_deref(), Some("202600011"));
    let bank = p.bank.as_ref().expect("bank");
    assert_eq!(bank.account_number.as_deref(), Some("19-2000145399/0800"));
    assert_eq!(bank.iban.as_deref(), Some("CZ6508000000192000145399"));
    assert!(p.warnings.is_empty(), "PaidAmount is ignored silently");
    assert_eq!(p.totals.payable, d("24000.00"));
    assert_eq!(p.totals.recap.len(), 1);
    let LineData::Item(i) = &p.lines[1] else {
        panic!("item expected");
    };
    assert_eq!((i.quantity, i.unit_price), (d("3"), d("3000")));
    assert_eq!(i.unit.as_deref(), Some("h"));
}

#[test]
fn received_vat_recap_lines_and_text() {
    let p = plan_of(VAT).expect("plan");
    assert_eq!(p.vat_mode, VatMode::Standard);
    assert_eq!(p.totals.recap[0].vat_rate, d("21"));
    assert_eq!(
        (p.totals.base, p.totals.vat, p.totals.total),
        (d("1100.00"), d("222.00"), d("1322.00"))
    );
    assert!(matches!(p.lines[2], LineData::Text { .. }));
    assert_eq!(p.constant_symbol.as_deref(), Some("0308"));
    assert_eq!(p.note.as_deref(), Some("Děkujeme za objednávku"));
}

#[test]
fn credit_notes_are_stored_positive() {
    let neg = CREDIT
        .replace(
            "<TaxInclusiveAmount>1322.00",
            "<TaxInclusiveAmount>-1322.00",
        )
        .replace(
            "<DifferenceTaxInclusiveAmount>1322.00",
            "<DifferenceTaxInclusiveAmount>-1322.00",
        )
        .replace(
            "<DifferenceTaxExclusiveAmount>1100.00",
            "<DifferenceTaxExclusiveAmount>-1100.00",
        )
        .replace("<PayableAmount>1322.00", "<PayableAmount>-1322.00")
        .replace(
            "<DifferenceTaxableAmount>1000.00",
            "<DifferenceTaxableAmount>-1000.00",
        )
        .replace(
            "<DifferenceTaxAmount>210.00",
            "<DifferenceTaxAmount>-210.00",
        )
        .replace(
            "<LineExtensionAmount>1000.00",
            "<LineExtensionAmount>-1000.00",
        )
        .replace("<UnitPrice>500.00", "<UnitPrice>-500.00");
    let p = plan_of(&neg).expect("plan");
    assert_eq!(p.doc_type, DocType::CreditNote);
    assert_eq!(p.totals.payable, d("1322.00"));
    assert_eq!(p.totals.total, d("1322.00"));
    assert_eq!(p.totals.recap[0].base, d("1000.00"));
    assert_eq!(p.totals.recap[0].vat, d("210.00"));
    let LineData::Item(i) = &p.lines[0] else {
        panic!("item expected");
    };
    assert_eq!(i.unit_price, d("500"));
    // Already positive (as the spec prescribes): unchanged.
    let pos = plan_of(CREDIT).expect("plan");
    assert_eq!(pos.totals.payable, d("1322.00"));
    assert_eq!(pos.original_ref.as_deref(), Some("FV-2026/077"));
}

#[test]
fn foreign_currency_reverse_charge() {
    let p = plan_of(EUR).expect("plan");
    assert_eq!(p.currency, "EUR");
    assert_eq!(p.rate, Some(d("24.335")));
    assert!(!p.needs_cnb());
    assert_eq!(p.vat_mode, VatMode::ReverseCharge);
    assert_eq!(p.totals.recap[0].base, d("1000.00"));
    assert_eq!(p.totals.recap[0].base_czk, Some(d("24335.00")));
    assert_eq!(p.totals.total_czk, Some(d("24335.00")));
    assert_eq!(p.payment_method, PaymentMethod::Other);
    assert_eq!(p.bank, None);
    let LineData::Item(i) = &p.lines[0] else {
        panic!("item expected");
    };
    assert_eq!(i.unit_price, d("100"), "base / quantity in EUR");
    let no_rate = EUR.replace("<CurrRate>24.335</CurrRate>", "<CurrRate>1</CurrRate>");
    let p = plan_of(&no_rate).expect("plan");
    assert!(p.needs_cnb());
    assert_eq!(p.warnings, [RATE_FROM_CNB]);
}

#[test]
fn vat_modes() {
    let received_zero = NONPAYER
        .replace(
            "<ID>44444443</ID></PartyIdentification><PartyName><Name>Dodavatel",
            "<ID>87654326</ID></PartyIdentification><PartyName><Name>Dodavatel",
        )
        .replace("<ID>12345679</ID>", "<ID>44444443</ID>");
    assert_eq!(
        plan_of(&received_zero).expect("plan").vat_mode,
        VatMode::Exempt
    );
    let flagged = VAT.replace(
        "<Percent>12</Percent><VATApplicable>true</VATApplicable>",
        "<Percent>12</Percent><VATApplicable>true</VATApplicable><LocalReverseChargeFlag>true</LocalReverseChargeFlag>",
    );
    assert_eq!(
        plan_of(&flagged).expect("plan").vat_mode,
        VatMode::ReverseCharge
    );
    let no_vat = VAT
        .replace("<DifferenceTaxAmount>210.00", "<DifferenceTaxAmount>0")
        .replace("<DifferenceTaxAmount>12.00", "<DifferenceTaxAmount>0");
    assert_eq!(plan_of(&no_vat).expect("plan").vat_mode, VatMode::Exempt);
}

#[test]
fn discount_from_before_discount_amount() {
    let x = VAT.replace(
        "<LineExtensionAmount>100.00</LineExtensionAmount>",
        "<LineExtensionAmount>90.00</LineExtensionAmount><LineExtensionAmountBeforeDiscount>100.00</LineExtensionAmountBeforeDiscount>",
    );
    let p = plan_of(&x).expect("plan");
    let LineData::Item(i) = &p.lines[1] else {
        panic!("item expected");
    };
    assert_eq!((i.unit_price, i.discount_pct), (d("100"), d("10")));
}

#[test]
fn proforma_has_no_tax_point_and_received_ddpp_may_lack_due_date() {
    let pro = VAT.replace(
        "<DocumentType>1</DocumentType>",
        "<DocumentType>4</DocumentType>",
    );
    assert_eq!(plan_of(&pro).expect("plan").tax_point_date, None);
    let ddpp = VAT
        .replace(
            "<DocumentType>1</DocumentType>",
            "<DocumentType>5</DocumentType>",
        )
        .replace("<PaymentDueDate>2026-09-24</PaymentDueDate>", "");
    assert_eq!(plan_of(&ddpp).expect("plan").due_date, None);
}

#[test]
fn exempt_needs_vat_that_would_be_due() {
    // VAT rounding to 0 on a tiny standard base stays standard.
    let tiny = VAT
        .replace(
            "<DifferenceTaxableAmount>1000.00",
            "<DifferenceTaxableAmount>0.02",
        )
        .replace("<DifferenceTaxAmount>210.00", "<DifferenceTaxAmount>0")
        .replace(
            "<DifferenceTaxableAmount>100.00",
            "<DifferenceTaxableAmount>0.04",
        )
        .replace("<DifferenceTaxAmount>12.00", "<DifferenceTaxAmount>0");
    assert_eq!(plan_of(&tiny).expect("plan").vat_mode, VatMode::Standard);
    // A zero-value document stays standard.
    let zero = VAT
        .replace(
            "<DifferenceTaxableAmount>1000.00",
            "<DifferenceTaxableAmount>0",
        )
        .replace("<DifferenceTaxAmount>210.00", "<DifferenceTaxAmount>0")
        .replace(
            "<DifferenceTaxableAmount>100.00",
            "<DifferenceTaxableAmount>0",
        )
        .replace("<DifferenceTaxAmount>12.00", "<DifferenceTaxAmount>0");
    assert_eq!(plan_of(&zero).expect("plan").vat_mode, VatMode::Standard);
    // VATApplicable missing on a row: not ours, standard.
    let unflagged = VAT
        .replace("<DifferenceTaxAmount>210.00", "<DifferenceTaxAmount>0")
        .replace("<DifferenceTaxAmount>12.00", "<DifferenceTaxAmount>0")
        .replace(
            "<VATApplicable>true</VATApplicable></TaxCategory>",
            "</TaxCategory>",
        );
    assert_eq!(
        plan_of(&unflagged).expect("plan").vat_mode,
        VatMode::Standard
    );
}

#[test]
fn non_taxed_deposit_comes_off_the_zero_row() {
    let x = NONPAYER
        .replace(
            "<PaidDepositsAmount>0.0</PaidDepositsAmount>",
            "<PaidDepositsAmount>4000</PaidDepositsAmount>",
        )
        .replace(
            "<PayableAmount>24000.0</PayableAmount>",
            "<PayableAmount>20000</PayableAmount>",
        );
    let p = plan_of(&x).expect("plan");
    assert_eq!(p.totals.recap[0].base, d("20000.00"));
    assert_eq!(
        (p.totals.base, p.totals.total),
        (d("20000.00"), d("20000.00"))
    );
    assert_eq!(p.totals.payable, p.totals.total + p.totals.rounding);
    // No 0 % row: one is added with the negative deposit.
    let v = VAT
        .replace(
            "<PaidDepositsAmount>0</PaidDepositsAmount>",
            "<PaidDepositsAmount>100</PaidDepositsAmount>",
        )
        .replace(
            "<PayableAmount>1322.00</PayableAmount>",
            "<PayableAmount>1222.00</PayableAmount>",
        );
    let p = plan_of(&v).expect("plan");
    assert_eq!(p.totals.recap.len(), 3);
    assert_eq!(p.totals.recap[2].vat_rate, Decimal::ZERO);
    assert_eq!(p.totals.recap[2].base, d("-100.00"));
    assert_eq!(p.totals.total, d("1222.00"));
    assert_eq!(p.totals.payable, p.totals.total);
}
