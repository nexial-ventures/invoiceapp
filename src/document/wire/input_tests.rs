use chrono::{NaiveDate, Utc};

use std::collections::HashMap;

use rust_decimal::Decimal;

use super::*;
use crate::contact::entity::contact;
use crate::document::model::Existing;
use crate::document::wire::compute_input::{ComputeCtx, ComputeInput};
use crate::settings::entity::company;

fn date(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("date")
}

fn company(vat_payer: bool) -> company::Model {
    company::Model {
        space_id: Uuid::nil(),
        name: "Me".into(),
        ico: None,
        dic: None,
        vat_payer,
        street: String::new(),
        city: String::new(),
        zip: String::new(),
        country: "CZ".into(),
        email: None,
        phone: None,
        web: None,
        registration: None,
        default_due_days: 14,
        default_locale: "cs".into(),
        default_rounding_cash: RoundingMode::Unit,
        default_rounding_other: RoundingMode::None,
        default_payment_method: PaymentMethod::BankTransfer,
        default_currency: "CZK".into(),
        default_header_note_cs: None,
        default_header_note_en: None,
        default_footer_note_cs: None,
        default_footer_note_en: None,
        updated_at: Utc::now().into(),
    }
}

pub(super) fn ctx(apply_defaults: bool, contact: Option<contact::Model>) -> Context {
    Context {
        today: date(10, 8),
        apply_defaults,
        company: company(true),
        contact,
        default_vat_rate: Some(Decimal::from(21)),
        existing: None,
        advances: HashMap::new(),
        related: None,
        meta: Default::default(),
        exact: None,
    }
}

pub(super) fn contact_with_defaults() -> contact::Model {
    contact::Model {
        id: Uuid::from_u128(7),
        space_id: Uuid::nil(),
        name: "C".into(),
        ico: None,
        dic: None,
        street: String::new(),
        city: String::new(),
        zip: String::new(),
        country: "CZ".into(),
        email: None,
        phone: None,
        note: None,
        default_due_days: Some(30),
        default_locale: Some("en".into()),
        default_currency: Some("EUR".into()),
        number_code: None,
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    }
}

pub(super) fn fields(err: AppError) -> FieldErrors {
    match err {
        AppError::Validation(f) => f,
        other => panic!("expected validation, got {other:?}"),
    }
}

#[test]
fn post_defaults_from_company() {
    let (d, ev) = DocumentInput::default()
        .validate(&ctx(true, None))
        .expect("valid");
    assert_eq!(d.issue_date, date(10, 8));
    assert_eq!(d.tax_point_date, Some(date(10, 8)));
    assert_eq!(d.due_date, date(10, 22));
    assert_eq!((d.currency.as_str(), d.locale.as_str()), ("CZK", "cs"));
    assert_eq!(d.vat_mode, VatMode::Standard);
    assert_eq!(d.payment_method, PaymentMethod::BankTransfer);
    assert_eq!(d.rounding_mode, RoundingMode::None);
    assert!(ev.totals.recap.is_empty());
}

#[test]
fn post_prefers_contact_defaults() {
    let c = contact_with_defaults();
    let input = DocumentInput {
        contact_id: Some(c.id),
        issue_date: Some(date(10, 1)),
        exchange_rate: Some("25.1".into()),
        ..Default::default()
    };
    let (d, _) = input.validate(&ctx(true, Some(c))).expect("valid");
    assert_eq!(d.due_date, date(10, 31));
    assert_eq!((d.currency.as_str(), d.locale.as_str()), ("EUR", "en"));
    assert_eq!(d.exchange_rate, Some("25.1".parse().expect("d")));
}

#[test]
fn put_requires_fields_and_keeps_nulls() {
    let err = DocumentInput::default()
        .validate(&ctx(false, None))
        .expect_err("required");
    let mut expected = FieldErrors::new();
    for f in [
        "issueDate",
        "dueDate",
        "currency",
        "locale",
        "vatMode",
        "paymentMethod",
    ] {
        expected.add(f, "required");
    }
    assert_eq!(fields(err), expected);
}

#[test]
fn rejects_unknown_contact_and_other_doc_types() {
    let input = DocumentInput {
        contact_id: Some(Uuid::from_u128(1)),
        direction: Some("received".into()),
        exchange_rate: Some("0".into()),
        ..Default::default()
    };
    let mut expected = FieldErrors::new();
    expected.add("docType", "invalid");
    expected.add("contactId", "invalid");
    expected.add("exchangeRate", "invalid");
    assert_eq!(
        fields(input.validate(&ctx(true, None)).expect_err("e")),
        expected
    );
}

#[test]
fn czk_ignores_the_exchange_rate() {
    let input = DocumentInput {
        currency: Some("czk".into()),
        exchange_rate: Some("25".into()),
        ..Default::default()
    };
    let (d, _) = input.validate(&ctx(true, None)).expect("valid");
    assert_eq!((d.currency.as_str(), d.exchange_rate), ("CZK", None));
}

#[test]
fn compute_input_defaults() {
    let input = ComputeInput {
        lines: vec![LineInput {
            kind: "item".into(),
            quantity: Some("2".into()),
            unit_price: Some("10".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cctx = ComputeCtx {
        vat_payer: true,
        default_rate: Some(Decimal::from(21)),
        default_locale: "cs".into(),
        existing: None,
        advances: HashMap::new(),
        exact: None,
    };
    let (lines, ev) = input.clone().validate(&cctx).expect("valid");
    assert_eq!(lines.len(), 1);
    assert_eq!(ev.totals.vat, "4.20".parse().expect("d"));
    cctx.vat_payer = false;
    let (_, ev) = input.validate(&cctx).expect("valid");
    assert_eq!(
        ev.totals.vat,
        Decimal::ZERO,
        "non-payer company → 0 % lines"
    );
}

fn existing(doc_type: DocType) -> Existing {
    Existing {
        id: Uuid::from_u128(42),
        doc_type,
        imported: false,
        related_document_id: Some(Uuid::from_u128(41)),
        contact_id: None,
        vat_mode: VatMode::Standard,
        currency: "EUR".into(),
        locale: "cs".into(),
        exchange_rate: Some("25.5".parse().expect("d")),
        number: None,
    }
}

#[test]
fn proforma_has_no_tax_point_date() {
    let input = DocumentInput {
        doc_type: Some("proforma".into()),
        ..Default::default()
    };
    let (d, _) = input.clone().validate(&ctx(true, None)).expect("valid");
    assert_eq!((d.doc_type, d.tax_point_date), (DocType::Proforma, None));
    let input = DocumentInput {
        tax_point_date: Some(date(10, 8)),
        ..input
    };
    let mut expected = FieldErrors::new();
    expected.add("taxPointDate", "invalid");
    assert_eq!(
        fields(input.validate(&ctx(true, None)).expect_err("e")),
        expected
    );
}

#[test]
fn doc_type_rules() {
    let mut expected = FieldErrors::new();
    expected.add("docType", "invalid");
    for t in ["credit_note", "advance_tax_doc", "bogus"] {
        let input = DocumentInput {
            doc_type: Some(t.into()),
            ..Default::default()
        };
        assert_eq!(
            fields(input.validate(&ctx(true, None)).expect_err("e")),
            expected,
            "{t}"
        );
    }
    // Update: omitted keeps the draft's type; another type is refused.
    let mut c = ctx(true, None);
    c.existing = Some(existing(DocType::Proforma));
    let input = DocumentInput {
        currency: Some("EUR".into()),
        ..Default::default()
    };
    let (d, _) = input.clone().validate(&c).expect("valid");
    assert_eq!(d.doc_type, DocType::Proforma);
    assert_eq!(d.related_document_id, Some(Uuid::from_u128(41)));
    let other = DocumentInput {
        doc_type: Some("invoice".into()),
        ..input
    };
    assert_eq!(fields(other.validate(&c).expect_err("e")), expected);
}

#[test]
fn credit_note_keeps_the_original_rate_and_currency() {
    let mut c = ctx(true, None);
    c.existing = Some(existing(DocType::CreditNote));
    let input = DocumentInput {
        currency: Some("EUR".into()),
        exchange_rate: Some("30".into()),
        correction_reason: Some(" Vrácení zboží ".into()),
        ..Default::default()
    };
    let (d, _) = input.clone().validate(&c).expect("valid");
    assert_eq!(d.exchange_rate, Some("25.5".parse().expect("d")));
    assert_eq!(d.correction_reason.as_deref(), Some("Vrácení zboží"));
    let usd = DocumentInput {
        currency: Some("USD".into()),
        vat_mode: Some("exempt".into()),
        correction_reason: Some("x".repeat(501)),
        ..input
    };
    let mut expected = FieldErrors::new();
    expected.add("currency", "invalid");
    expected.add("vatMode", "invalid");
    expected.add("correctionReason", "too_long");
    assert_eq!(fields(usd.validate(&c).expect_err("e")), expected);
}

#[test]
fn ddpp_correction_save_applies_the_exact_vat() {
    use crate::document::correction::ExactBasis;
    use crate::document::line::AdvanceRow;
    let mut c = ctx(true, None);
    c.existing = Some(existing(DocType::AdvanceCreditNote));
    let d = |s: &str| s.parse::<Decimal>().expect("d");
    c.exact = Some(ExactBasis {
        ddpp: vec![AdvanceRow {
            vat_rate: d("21"),
            base: d("564.98"),
            vat: d("118.64"),
            base_czk: None,
            vat_czk: None,
        }],
        credited: vec![],
    });
    let input = DocumentInput {
        currency: Some("EUR".into()),
        lines: vec![LineInput {
            kind: "item".into(),
            description: "Záloha".into(),
            quantity: Some("1".into()),
            unit_price: Some("564.98".into()),
            vat_rate: Some("21".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let (_, ev) = input.validate(&c).expect("valid");
    assert_eq!(
        ev.totals.vat,
        d("118.64"),
        "not round2(564.98 × 21 %) = 118.65"
    );
}
