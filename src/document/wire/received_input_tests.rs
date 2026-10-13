use serde_json::{Value, json};

use crate::error::FieldErrors;

use super::*;
use crate::contact::entity::contact;

fn contact() -> contact::Model {
    let now = chrono::Utc::now().into();
    contact::Model {
        id: Uuid::from_u128(1),
        space_id: Uuid::nil(),
        name: "Dodavatel Test s.r.o.".into(),
        ico: None,
        dic: None,
        street: String::new(),
        city: "Praha".into(),
        zip: String::new(),
        country: "CZ".into(),
        email: None,
        phone: None,
        default_due_days: None,
        default_locale: None,
        default_currency: None,
        number_code: None,
        note: None,
        created_at: now,
        updated_at: now,
    }
}

fn ctx() -> ReceivedCtx {
    ReceivedCtx {
        contact: Some(contact()),
        ..Default::default()
    }
}

fn body(extra: Value) -> ReceivedInput {
    let mut b = json!({
        "direction": "received", "supplierNumber": "FV-1", "contactId": Uuid::from_u128(1),
        "issueDate": "2026-10-01", "taxPointDate": "2026-10-01", "dueDate": "2026-10-15",
        "vatRecap": [{ "rate": "21", "base": "1000", "vat": "210" }], "payable": "1210"
    });
    if let (Some(o), Some(e)) = (b.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(b).expect("body")
}

fn fields(r: Result<ReceivedData, AppError>) -> FieldErrors {
    match r {
        Err(AppError::Validation(f)) => f,
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[test]
fn defaults_on_create() {
    let d = body(json!({})).validate(&ctx()).expect("valid");
    assert_eq!(d.doc_type, DocType::Invoice);
    assert_eq!(d.currency, "CZK");
    assert_eq!(d.vat_mode, VatMode::Standard);
    assert!(d.vat_deductible);
    assert_eq!(d.received_date, d.tax_point_date.expect("tax point"));
    assert_eq!(d.rounding, Decimal::ZERO);
    let d = body(
        json!({ "vatMode": "non_payer", "taxPointDate": null, "docType": "proforma",
        "vatRecap": [{ "rate": "0", "base": "100", "vat": "0" }] }),
    )
    .validate(&ctx())
    .expect("valid");
    assert!(!d.vat_deductible);
    assert_eq!(d.received_date, d.issue_date);
}

#[test]
fn required_fields() {
    let f = fields(
        ReceivedInput {
            direction: Some("received".into()),
            ..Default::default()
        }
        .validate(&ctx()),
    );
    for k in [
        "supplierNumber",
        "contactId",
        "issueDate",
        "taxPointDate",
        "dueDate",
        "vatRecap",
        "payable",
    ] {
        assert_eq!(f.get(k), Some("required"), "{k}: {f:?}");
    }
}

#[test]
fn per_type_dates() {
    let f = fields(body(json!({ "docType": "proforma" })).validate(&ctx()));
    assert_eq!(f.get("taxPointDate"), Some("invalid"));
    let d = body(json!({ "docType": "advance_tax_doc", "dueDate": null }))
        .validate(&ctx())
        .expect("DDPP without due date");
    assert_eq!(d.due_date, None);
    let f = fields(body(json!({ "docType": "received" })).validate(&ctx()));
    assert_eq!(f.get("docType"), Some("invalid"));
}

#[test]
fn recap_rows() {
    let f = fields(
        body(json!({ "vatRecap": [
            { "rate": "21", "base": "1", "vat": "0.001" },
            { "rate": "21", "base": "1", "vat": "0" },
            { "rate": "101", "base": "x", "vat": "0" },
        ] }))
        .validate(&ctx()),
    );
    assert_eq!(f.get("vatRecap.0.vat"), Some("invalid"));
    assert_eq!(f.get("vatRecap.1.rate"), Some("duplicate"));
    assert_eq!(f.get("vatRecap.2.rate"), Some("invalid"));
    assert_eq!(f.get("vatRecap.2.base"), Some("invalid"));
    // Any rate is fine (foreign / old rates).
    let d = body(json!({ "vatRecap": [{ "rate": "19.5", "base": "100", "vat": "19.5" }] }))
        .validate(&ctx())
        .expect("valid");
    assert_eq!(d.recap[0].rate, "19.5".parse::<Decimal>().expect("d"));
    let f = fields(body(json!({ "vatMode": "reverse_charge" })).validate(&ctx()));
    assert_eq!(f.get("vatRecap.0.vat"), Some("invalid"));
}

#[test]
fn amounts() {
    let f = fields(body(json!({ "rounding": "100", "payable": "-1" })).validate(&ctx()));
    assert_eq!(f.get("rounding"), Some("invalid"));
    assert_eq!(f.get("payable"), Some("invalid"));
    let f = fields(
        body(json!({ "vatRecap": [
            { "rate": "21", "base": "10000000000000000", "vat": "0" },
        ] }))
        .validate(&ctx()),
    );
    assert_eq!(f.get("vatRecap.0.base"), Some("invalid"));
    let f = fields(
        body(json!({ "vatRecap": [
            { "rate": "21", "base": "9000000000000000", "vat": "0" },
            { "rate": "12", "base": "9000000000000000", "vat": "0" },
        ] }))
        .validate(&ctx()),
    );
    assert_eq!(f.get("vatRecap"), Some("invalid"));
}

#[test]
fn put_keeps_type_and_requires_currency() {
    let mut c = ctx();
    c.existing = Some((Uuid::from_u128(9), DocType::CreditNote));
    let f = fields(body(json!({ "docType": "invoice" })).validate(&c));
    assert_eq!(f.get("docType"), Some("invalid"));
    assert_eq!(f.get("currency"), Some("required"));
    assert_eq!(f.get("vatMode"), Some("required"));
    let d = body(json!({ "currency": "EUR", "vatMode": "standard", "exchangeRate": "24.5" }))
        .validate(&c)
        .expect("valid");
    assert_eq!(d.doc_type, DocType::CreditNote);
    assert_eq!(d.exchange_rate, Some("24.5".parse().expect("d")));
}

#[test]
fn text_limits_and_contact() {
    let mut c = ctx();
    c.contact = None;
    let f = fields(
        body(
            json!({ "supplierNumber": "x".repeat(41), "supplierAccount": "y".repeat(61),
            "variableSymbol": "12a" }),
        )
        .validate(&c),
    );
    assert_eq!(f.get("supplierNumber"), Some("too_long"));
    assert_eq!(f.get("supplierAccount"), Some("too_long"));
    assert_eq!(f.get("variableSymbol"), Some("invalid"));
    assert_eq!(f.get("contactId"), Some("invalid"));
    let f = fields(body(json!({ "relatedDocumentId": Uuid::from_u128(5) })).validate(&ctx()));
    assert_eq!(f.get("relatedDocumentId"), Some("invalid"));
}
