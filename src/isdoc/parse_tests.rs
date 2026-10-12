use super::*;
use crate::document::doc_type::DocType;

pub const NONPAYER: &str = include_str!("../../tests/fixtures/isdoc/issued_nonpayer.isdoc");
pub const VAT: &str = include_str!("../../tests/fixtures/isdoc/received_vat.isdoc");
pub const CREDIT: &str = include_str!("../../tests/fixtures/isdoc/received_credit.isdoc");
pub const EUR: &str = include_str!("../../tests/fixtures/isdoc/received_eur.isdoc");

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

fn day(s: &str) -> NaiveDate {
    s.parse().expect("date")
}

#[test]
fn reads_a_non_payer_invoice() {
    let p = parse(NONPAYER.as_bytes()).expect("parsed");
    assert_eq!(p.doc_type, DocType::Invoice);
    assert_eq!(p.number, "202600011");
    assert_eq!(p.issue_date, day("2026-10-01"));
    assert_eq!(p.tax_point_date, None);
    assert!(!p.vat_applicable);
    assert_eq!(p.currency, "CZK");
    assert_eq!(p.rate, None);
    assert_eq!(p.supplier.ico.as_deref(), Some("44444443"));
    let c = p.customer.expect("customer");
    assert_eq!(c.ico.as_deref(), Some("12345679"));
    assert_eq!(c.dic, None);
    assert_eq!(
        (c.city.as_str(), c.zip.as_str(), c.country.as_str()),
        ("Brno", "60200", "CZ")
    );
    assert_eq!(p.lines.len(), 2);
    assert_eq!(p.lines[1].quantity, Some(d("3")));
    assert_eq!(p.lines[1].unit.as_deref(), Some("h"));
    assert_eq!(p.lines[1].unit_price, d("3000"));
    assert_eq!(p.recap.len(), 1);
    assert_eq!(p.recap[0].vat_applicable, Some(false));
    assert_eq!(p.totals.payable.doc, d("24000"));
    assert_eq!(p.totals.gross, d("24000"));
    let pay = p.payment.expect("payment");
    assert_eq!(pay.code.as_deref(), Some("42"));
    assert_eq!(pay.due_date, Some(day("2026-10-16")));
    assert_eq!(pay.account.as_deref(), Some("19-2000145399"));
    assert_eq!(pay.bank_code.as_deref(), Some("0800"));
    assert_eq!(pay.bic, None);
    assert_eq!(pay.constant_symbol, None);
}

#[test]
fn reads_parties_recap_and_text_lines() {
    let p = parse(VAT.as_bytes()).expect("parsed");
    assert_eq!(p.tax_point_date, Some(day("2026-09-08")));
    assert_eq!(
        p.supplier.ico.as_deref(),
        Some("87654326"),
        "whitespace stripped"
    );
    assert_eq!(p.supplier.street, "Vymyšlená 7");
    assert_eq!(p.supplier.dic.as_deref(), Some("CZ87654326"));
    assert_eq!(
        p.supplier.registration.as_deref(),
        Some("B 999 vedená u KS v Ostravě")
    );
    assert_eq!(p.supplier.email.as_deref(), Some("fakturace@example.com"));
    assert_eq!(p.note.as_deref(), Some("Děkujeme za objednávku"));
    assert_eq!(p.recap.len(), 2);
    assert_eq!((p.recap[1].rate, p.recap[1].vat.doc), (d("12"), d("12")));
    assert_eq!(p.lines[2].quantity, None);
    assert_eq!(p.lines[2].base.doc, Decimal::ZERO);
    assert_eq!(p.original_ref, None);
    let c = parse(CREDIT.as_bytes()).expect("credit");
    assert_eq!(c.doc_type, DocType::CreditNote);
    assert_eq!(c.original_ref.as_deref(), Some("FV-2026/077"));
}

#[test]
fn foreign_currency_takes_curr_amounts_and_the_rate() {
    let p = parse(EUR.as_bytes()).expect("parsed");
    assert_eq!(p.currency, "EUR");
    assert_eq!(p.rate, Some(d("24.335")));
    assert_eq!(p.recap[0].base.doc, d("1000"));
    assert_eq!(p.recap[0].base.czk, Some(d("24335")));
    assert_eq!(p.totals.payable.czk, Some(d("24335")));
    assert_eq!(p.lines[0].base.doc, d("1000"));
    assert_eq!(p.payment, None);
}

#[test]
fn rate_of_one_or_missing_curr_is_unusable() {
    let one = EUR.replace("<CurrRate>24.335</CurrRate>", "<CurrRate>1</CurrRate>");
    assert_eq!(parse(one.as_bytes()).expect("parsed").rate, None);
    let per_100 = EUR
        .replace("<CurrRate>24.335</CurrRate>", "<CurrRate>2433.5</CurrRate>")
        .replace(
            "<RefCurrRate>1</RefCurrRate>",
            "<RefCurrRate>100</RefCurrRate>",
        );
    assert_eq!(
        parse(per_100.as_bytes()).expect("parsed").rate,
        Some(d("24.335"))
    );
    let no_curr = EUR.replace("<PayableAmountCurr>1000.00</PayableAmountCurr>", "");
    assert_eq!(parse(no_curr.as_bytes()), Err(MISSING_FIELD));
}

#[test]
fn error_codes() {
    assert_eq!(parse(b"not xml"), Err(INVALID_XML));
    assert_eq!(parse(b"<Other/>"), Err(INVALID_XML));
    assert_eq!(
        parse(br#"<!DOCTYPE x [<!ENTITY a "aaaa">]><Invoice/>"#),
        Err(INVALID_XML),
        "DTDs are refused"
    );
    let v5 = VAT.replace("version=\"6.0.2\"", "version=\"5.2\"");
    assert_eq!(parse(v5.as_bytes()), Err(UNSUPPORTED_VERSION));
    let old_ns = VAT.replace("namespace/2013", "namespace/invoice");
    assert_eq!(parse(old_ns.as_bytes()), Err(UNSUPPORTED_VERSION));
    let t9 = VAT.replace(
        "<DocumentType>1</DocumentType>",
        "<DocumentType>9</DocumentType>",
    );
    assert_eq!(parse(t9.as_bytes()), Err(UNSUPPORTED_TYPE));
    let no_id = VAT.replace("<ID>FV-2026/077</ID>", "");
    assert_eq!(parse(no_id.as_bytes()), Err(MISSING_FIELD));
    let long = VAT.replace("FV-2026/077", &"9".repeat(41));
    assert_eq!(parse(long.as_bytes()), Err(MISSING_FIELD));
    let huge = VAT.replace(
        "<PayableAmount>1322.00</PayableAmount>",
        "<PayableAmount>1000000000000</PayableAmount>",
    );
    assert_eq!(parse(huge.as_bytes()), Err(INVALID_AMOUNT));
    let junk = VAT.replace(
        "<PayableAmount>1322.00</PayableAmount>",
        "<PayableAmount>12,5</PayableAmount>",
    );
    assert_eq!(parse(junk.as_bytes()), Err(INVALID_AMOUNT));
    let eur_local = VAT.replace(
        "<LocalCurrencyCode>CZK</LocalCurrencyCode>",
        "<LocalCurrencyCode>EUR</LocalCurrencyCode>",
    );
    assert_eq!(parse(eur_local.as_bytes()), Err(UNSUPPORTED_CURRENCY));
    let rate = VAT.replace("<Percent>12</Percent>", "<Percent>120</Percent>");
    assert_eq!(parse(rate.as_bytes()), Err(INVALID_AMOUNT));
}

#[test]
fn paid_deposits_and_preview_file() {
    let p = parse(NONPAYER.as_bytes()).expect("parsed");
    assert_eq!(p.totals.paid_deposits.doc, Decimal::ZERO);
    let x = NONPAYER.replace(
        "<PaidDepositsAmount>0.0</PaidDepositsAmount>",
        "<PaidDepositsAmount>400</PaidDepositsAmount>",
    );
    assert_eq!(
        parse(x.as_bytes())
            .expect("parsed")
            .totals
            .paid_deposits
            .doc,
        d("400")
    );
    assert_eq!(preview_file_of(NONPAYER.as_bytes()), None);
    assert_eq!(preview_file_of(b"junk"), None);
}
