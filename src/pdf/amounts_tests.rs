//! Unit tests of the amount parts: negation, VAT modes, recaps, totals.

use uuid::Uuid;

use crate::document::line::VatMode;
use crate::document::wire::dto::Line;
use crate::document::wire::line_out::{AdvanceLine, AdvanceRecap};
use crate::pdf::format::Locale;
use crate::pdf::payload::*;
use crate::pdf::test_fixtures::*;

#[test]
fn credit_note_is_negated() {
    let f = Fixture::new(
        vec![item(1, "100", "0", "21", "200")],
        totals("200", "42", "0", "242"),
    );
    let mut i = f.input();
    i.doc_type = "credit_note";
    i.parent_number = Some("20260001");
    i.correction_reason = Some("Reklamace");
    let p = build(&i).expect("payload");
    assert_eq!(p.title, "Opravný daňový doklad");
    assert_eq!(
        p.reference.as_deref(),
        Some("Opravný daňový doklad k faktuře 20260001\nDůvod opravy: Reklamace")
    );
    assert_eq!(p.lines[0].unit_price, Some(format!("-100,00{NB}Kč")));
    assert_eq!(p.lines[0].base, Some(format!("-200,00{NB}Kč")));
    assert_eq!(p.lines[0].quantity, Some(format!("2{NB}ks")));
    let recap = p.vat_recap.as_ref().expect("recap");
    assert_eq!(recap.rows[0].vat, format!("-42,00{NB}Kč"));
    assert_eq!(recap.total.total, format!("-242,00{NB}Kč"));
    assert_eq!(
        p.totals.last().map(|t| t.value.as_str()),
        Some("-242,00\u{a0}Kč")
    );
    assert_eq!(p.qr, None);
}

#[test]
fn non_payer_hides_vat() {
    let f = Fixture::new(
        vec![item(1, "100", "0", "0", "200")],
        totals("200", "0", "0", "200"),
    );
    let mut i = f.input();
    i.vat_mode = VatMode::NonPayer;
    let p = build(&i).expect("payload");
    assert_eq!(p.title, "Faktura");
    assert!(!p.show_vat);
    assert_eq!(p.vat_recap, None);
    assert_eq!(p.columns.base, "Celkem");
    assert_eq!(p.lines[0].vat_rate, None);
    assert_eq!(p.dates.len(), 2, "no tax point for a non-payer");
    assert_eq!(p.legal_note.as_deref(), Some("Dodavatel není plátcem DPH."));
    assert_eq!(
        values(&p.totals),
        [
            ("Celkem", "200,00\u{a0}Kč", false),
            ("K úhradě", "200,00\u{a0}Kč", true)
        ]
    );
}

#[test]
fn foreign_currency_has_czk_recap() {
    let mut t = totals("100", "21", "0", "121");
    t.recap[0].base_czk = Some(d("2440"));
    t.recap[0].vat_czk = Some(d("512.40"));
    let f = Fixture::new(vec![item(1, "50", "0", "21", "100")], t);
    let mut i = f.input();
    i.currency = "EUR";
    i.locale = Locale::En;
    i.rate = Some(RateInfo {
        rate: d("24.4"),
        cnb_date: Some(day(10, 7)),
    });
    let p = build(&i).expect("payload");
    assert_eq!(p.title, "Invoice – tax document");
    assert_eq!(p.lines[0].base, Some(format!("EUR{NB}100.00")));
    let czk = p.vat_recap_czk.as_ref().expect("CZK recap");
    assert_eq!(czk.title, "VAT summary in CZK");
    assert_eq!(
        czk.rate_note.as_deref(),
        Some("ČNB rate 24.400 CZK/EUR of 7 Oct 2026")
    );
    assert_eq!(czk.rows[0].vat, format!("CZK{NB}512.40"));
    assert_eq!(czk.total.total, format!("CZK{NB}2,952.40"));

    i.rate = Some(RateInfo {
        rate: d("24.4"),
        cnb_date: None,
    });
    i.locale = Locale::Cs;
    let p = build(&i).expect("payload");
    let note = p.vat_recap_czk.and_then(|r| r.rate_note);
    assert_eq!(note.as_deref(), Some("Kurz 24,400 CZK/EUR"));

    i.vat_mode = VatMode::ReverseCharge;
    assert_eq!(build(&i).expect("payload").vat_recap_czk, None);
}

#[test]
fn ddpp_reference_paid_note_and_dates() {
    let f = Fixture::new(
        vec![item(1, "1000", "0", "21", "1000")],
        totals("1000", "210", "0", "1210"),
    );
    let mut i = f.input();
    i.doc_type = "advance_tax_doc";
    i.parent_number = Some("Z20260003");
    let p = build(&i).expect("payload");
    assert_eq!(p.title, "Daňový doklad k přijaté platbě");
    assert_eq!(p.reference.as_deref(), Some("K zálohové faktuře Z20260003"));
    assert_eq!(p.paid_note.as_deref(), Some("Neplaťte – již uhrazeno."));
    let dates: Vec<&str> = p.dates.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(
        dates,
        [
            "Datum vystavení",
            "Datum zdanitelného plnění",
            "Datum přijetí platby"
        ]
    );
    assert_eq!(p.qr, None);
    assert_eq!(
        values(&p.totals).last().copied(),
        Some(("Celkem s DPH", "1\u{a0}210,00\u{a0}Kč", true))
    );
}

#[test]
fn proforma_has_no_tax_point_and_settled_invoice_shows_the_deduction() {
    let f = standard();
    let mut i = f.input();
    i.doc_type = "proforma";
    let p = build(&i).expect("payload");
    assert_eq!(p.title, "Zálohová faktura");
    assert_eq!(p.dates.len(), 2);
    assert!(p.qr.is_some());

    let adv = Line::Advance(AdvanceLine {
        position: 2,
        advance_document_id: Uuid::nil(),
        description: "Odpočet zálohy DP20260001".into(),
        base: d("-100"),
        recap: vec![AdvanceRecap {
            vat_rate: d("21"),
            base: d("-100"),
            vat: d("-21"),
        }],
    });
    let f = Fixture::new(
        vec![item(1, "100", "0", "21", "200"), adv],
        totals("100", "21", "0", "121"),
    );
    let mut i = f.input();
    i.parent_number = Some("Z20260001");
    let p = build(&i).expect("payload");
    assert_eq!(p.reference, None, "a settled invoice lists no reference");
    assert_eq!(p.lines[1].kind, "advance");
    assert_eq!(p.lines[1].base, Some(format!("-100,00{NB}Kč")));
    assert_eq!(
        values(&p.totals),
        [
            ("Celkem bez DPH", "200,00\u{a0}Kč", false),
            ("DPH", "42,00\u{a0}Kč", false),
            ("Celkem s DPH", "242,00\u{a0}Kč", false),
            ("Odpočet záloh", "-121,00\u{a0}Kč", false),
            ("K úhradě", "121,00\u{a0}Kč", true),
        ]
    );
}
