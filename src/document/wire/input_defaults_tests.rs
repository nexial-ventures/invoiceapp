//! 4e defaults on create (`docs/api/defaults.md`).

use super::tests::{contact_with_defaults, ctx, fields};
use super::*;

fn with_defaults(c: &mut Context) {
    c.company.default_payment_method = "cash".into();
    c.company.default_rounding_cash = "half".into();
    c.company.default_rounding_other = "unit".into();
    c.company.default_currency = "EUR".into();
    c.company.default_header_note_cs = Some("Záhlaví".into());
    c.company.default_header_note_en = Some("Header".into());
    c.company.default_footer_note_cs = Some("Zápatí".into());
}

#[test]
fn post_takes_the_company_defaults() {
    let mut c = ctx(true, None);
    with_defaults(&mut c);
    let (d, _) = DocumentInput::default().validate(&c).expect("valid");
    assert_eq!(d.payment_method, PaymentMethod::Cash);
    assert_eq!(d.rounding_mode, RoundingMode::Half);
    assert_eq!(d.currency, "EUR");
    assert_eq!(d.header_note.as_deref(), Some("Záhlaví"));
    assert_eq!(d.footer_note.as_deref(), Some("Zápatí"));
}

#[test]
fn rounding_follows_the_given_payment_method() {
    let mut c = ctx(true, None);
    with_defaults(&mut c);
    let input = DocumentInput {
        payment_method: Some("card".into()),
        ..Default::default()
    };
    let (d, _) = input.validate(&c).expect("valid");
    assert_eq!(d.rounding_mode, RoundingMode::Unit);
    // The seeded defaults: cash → unit, other → none.
    let c = ctx(true, None);
    let input = DocumentInput {
        payment_method: Some("cash".into()),
        ..Default::default()
    };
    assert_eq!(
        input.validate(&c).expect("valid").0.rounding_mode,
        RoundingMode::Unit
    );
    let (d, _) = DocumentInput::default().validate(&c).expect("valid");
    assert_eq!(d.rounding_mode, RoundingMode::None);
}

#[test]
fn contact_wins_and_notes_follow_the_locale() {
    let contact = contact_with_defaults();
    let mut c = ctx(true, Some(contact.clone()));
    with_defaults(&mut c);
    c.company.default_currency = "USD".into();
    let input = DocumentInput {
        contact_id: Some(contact.id),
        ..Default::default()
    };
    let (d, _) = input.validate(&c).expect("valid");
    // The contact's EUR and en win; en has no footer default.
    assert_eq!((d.currency.as_str(), d.locale.as_str()), ("EUR", "en"));
    assert_eq!(d.header_note.as_deref(), Some("Header"));
    assert_eq!(d.footer_note, None);
}

#[test]
fn explicit_values_win_over_defaults() {
    let mut c = ctx(true, None);
    with_defaults(&mut c);
    let input = DocumentInput {
        rounding_mode: Some("none".into()),
        header_note: Some("".into()),
        footer_note: Some("Vlastní".into()),
        locale: Some("en".into()),
        ..Default::default()
    };
    let (d, _) = input.validate(&c).expect("valid");
    assert_eq!(d.rounding_mode, RoundingMode::None);
    assert_eq!(
        d.header_note, None,
        "an explicit empty note is not defaulted"
    );
    assert_eq!(d.footer_note.as_deref(), Some("Vlastní"));
}

#[test]
fn put_applies_no_defaults() {
    let mut c = ctx(false, None);
    with_defaults(&mut c);
    let input = DocumentInput {
        issue_date: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 1).expect("date")),
        due_date: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 15).expect("date")),
        currency: Some("CZK".into()),
        locale: Some("cs".into()),
        vat_mode: Some("standard".into()),
        payment_method: Some("cash".into()),
        ..Default::default()
    };
    let (d, _) = input.validate(&c).expect("valid");
    assert_eq!(d.rounding_mode, RoundingMode::None);
    assert_eq!((d.header_note, d.footer_note), (None, None));
}

#[test]
fn unknown_mode_is_invalid() {
    let input = DocumentInput {
        rounding_mode: Some("whole".into()),
        ..Default::default()
    };
    let mut expected = FieldErrors::new();
    expected.add("roundingMode", "invalid");
    assert_eq!(
        fields(input.validate(&ctx(true, None)).expect_err("e")),
        expected
    );
    // Rounding is allowed for every currency now.
    let input = DocumentInput {
        currency: Some("EUR".into()),
        rounding_mode: Some("half".into()),
        ..Default::default()
    };
    assert!(input.validate(&ctx(true, None)).is_ok());
}

#[test]
fn a_corrupt_company_default_is_an_internal_error() {
    let mut c = ctx(true, None);
    c.company.default_rounding_other = "whole".into();
    let err = DocumentInput::default().validate(&c).expect_err("corrupt");
    assert!(matches!(err, AppError::Internal(_)), "{err:?}");
    // Not consulted when the request names the mode.
    let input = DocumentInput {
        rounding_mode: Some("unit".into()),
        ..Default::default()
    };
    assert!(input.validate(&c).is_ok());
}
