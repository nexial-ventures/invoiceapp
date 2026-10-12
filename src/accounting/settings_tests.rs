use serde_json::json;

use super::*;

#[test]
fn responses_carry_the_limits_put_enforces() {
    let r: AccountingResponse = AccountingSettings::default().full().into();
    let v = serde_json::to_value(&r).expect("json");
    assert_eq!(
        v["limits"],
        json!({ "pohoda": { "code": 19, "numberSeries": 19 },
                "money": { "code": 10, "numberSeries": 5 } })
    );
    assert_eq!(v["pohoda"]["codes"].as_array().map(Vec::len), Some(12));
}

fn parse(v: serde_json::Value) -> AccountingUpdate {
    serde_json::from_value(v).expect("shape")
}

fn errors(v: serde_json::Value) -> FieldErrors {
    match parse(v).validate() {
        Err(AppError::Validation(e)) => e,
        other => panic!("expected validation, got {other:?}"),
    }
}

fn valid(v: serde_json::Value) -> AccountingSettings {
    parse(v)
        .validate()
        .expect("valid")
        .apply(AccountingSettings::default())
}

#[test]
fn full_lists_every_row() {
    let s = AccountingSettings::default().full();
    for p in [&s.pohoda, &s.money] {
        assert_eq!(p.codes.len(), 12);
        assert_eq!(
            p.codes[0],
            CodeRow::empty(Direction::Issued, DocType::Invoice)
        );
        assert_eq!(
            p.codes[11],
            CodeRow::empty(Direction::Received, DocType::Simplified)
        );
    }
    let json = serde_json::to_value(&s).expect("json");
    assert_eq!(json["pohoda"]["ico"], json!(null));
    assert_eq!(json["money"]["ico"], json!(null));
    // A 3b row (no `money`) still decodes.
    let old: AccountingSettings =
        serde_json::from_value(json!({ "pohoda": { "ico": "44444443", "codes": [] } }))
            .expect("3b shape");
    assert_eq!(old.full().money.codes.len(), 12);
}

#[test]
fn normalizes() {
    let s = valid(json!({ "pohoda": { "ico": " 4444 4443 ", "codes": [
        { "direction": "received", "docType": "credit_note", "accounting": " 3Pdob ",
          "classificationVatNonDeductible": " PN ",
          "classificationVat": "", "numberSeries": null },
        { "direction": " issued", "docType": "invoice", "numberSeries": "FV" },
    ]}}));
    assert_eq!(s.pohoda.ico.as_deref(), Some("44444443"));
    assert_eq!(s.pohoda.codes.len(), 12);
    let fv = s
        .pohoda
        .row(Direction::Issued, DocType::Invoice)
        .expect("row");
    assert_eq!(fv.number_series.as_deref(), Some("FV"));
    let dob = s
        .pohoda
        .row(Direction::Received, DocType::CreditNote)
        .expect("row");
    assert_eq!(dob.accounting.as_deref(), Some("3Pdob"));
    assert_eq!(dob.classification_vat, None);
    assert_eq!(dob.classification_vat_non_deductible.as_deref(), Some("PN"));
    assert_eq!(s.pohoda.row(Direction::Issued, DocType::Proforma), None);
    assert_eq!(s.money.codes.len(), 12, "absent section → stored (empty)");
}

#[test]
fn rejects() {
    let e = errors(json!({ "pohoda": { "ico": "12345678", "codes": [
        { "direction": "both", "docType": "invoice" },
        { "direction": "issued", "docType": "proforma" },
        { "direction": "issued", "docType": "invoice", "accounting": "x".repeat(20) },
        { "direction": "issued", "docType": "invoice" },
        { "direction": "issued", "docType": "debit_note", "classificationVatNonDeductible": "PN" },
    ]}}));
    assert_eq!(e.get("pohoda.ico"), Some("invalid_ico"));
    assert_eq!(
        e.get("pohoda.codes.4.classificationVatNonDeductible"),
        Some("invalid"),
        "received only"
    );
    assert_eq!(e.get("pohoda.codes.0.direction"), Some("invalid"));
    assert_eq!(e.get("pohoda.codes.1.docType"), Some("invalid"));
    assert_eq!(e.get("pohoda.codes.2.accounting"), Some("too_long"));
    assert_eq!(e.get("pohoda.codes.3.docType"), Some("duplicate"));
    let ok = json!({ "pohoda": { "codes": [
        { "direction": "issued", "docType": "invoice", "accounting": "x".repeat(19),
          "numberSeries": "y".repeat(19) }]}});
    assert!(parse(ok).validate().is_ok());
}

#[test]
fn money_limits() {
    let x = |n: usize| "x".repeat(n);
    let e = errors(json!({ "money": { "ico": "1", "codes": [
        { "direction": "issued", "docType": "invoice", "accounting": x(11),
          "classificationVat": x(11), "numberSeries": x(6) },
        { "direction": "received", "docType": "invoice", "classificationVatNonDeductible": x(11) },
        { "direction": "issued", "docType": "simplified", "classificationVatNonDeductible": "PN" },
        { "direction": "received", "docType": "invoice" },
    ]}, "pohoda": { "codes": [{ "direction": "x", "docType": "invoice" }] }}));
    for (field, reason) in [
        ("money.ico", "invalid_ico"),
        ("money.codes.0.accounting", "too_long"),
        ("money.codes.0.classificationVat", "too_long"),
        ("money.codes.0.numberSeries", "too_long"),
        ("money.codes.1.classificationVatNonDeductible", "too_long"),
        ("money.codes.2.classificationVatNonDeductible", "invalid"),
        ("money.codes.3.docType", "duplicate"),
        ("pohoda.codes.0.direction", "invalid"),
    ] {
        assert_eq!(e.get(field), Some(reason), "{field}");
    }
    let s = valid(json!({ "money": { "ico": "87654326", "codes": [
        { "direction": "received", "docType": "advance_tax_doc", "accounting": x(10),
          "classificationVat": x(10), "classificationVatNonDeductible": x(10),
          "numberSeries": x(5) }]}}));
    assert_eq!(s.money.ico.as_deref(), Some("87654326"));
    let row = s
        .money
        .row(Direction::Received, DocType::AdvanceTaxDoc)
        .expect("row");
    assert_eq!(row.number_series.as_deref(), Some("xxxxx"));
    assert_eq!(s.section(Program::Money), &s.money);
}

#[test]
fn present_sections_replace_absent_ones_stay() {
    let stored = valid(json!({
        "pohoda": { "ico": "44444443", "codes": [] },
        "money": { "ico": "12345679", "codes": [
            { "direction": "issued", "docType": "invoice", "numberSeries": "FV" }] },
    }));
    let only_pohoda = parse(json!({ "pohoda": { "codes": [] } }))
        .validate()
        .expect("valid")
        .apply(stored.clone());
    assert_eq!(only_pohoda.pohoda.ico, None, "present → replaced");
    assert_eq!(only_pohoda.money, stored.money, "absent → kept");
    let nothing = parse(json!({})).validate().expect("valid");
    assert_eq!(nothing.apply(stored.clone()), stored);
    let cleared = parse(json!({ "money": {}, "pohoda": null }))
        .validate()
        .expect("valid")
        .apply(stored.clone());
    assert_eq!(cleared.money, ProgramSettings::default().full());
    assert_eq!(cleared.pohoda, stored.pohoda, "null = absent");
}
