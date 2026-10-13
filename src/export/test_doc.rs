//! A stored document for the export unit tests (fictitious parties).

use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

use crate::document::doc_type::DocType;
use crate::document::entity::{document, vat_recap};
use crate::document::line::{Direction, PaymentMethod, RoundingMode, Status, VatMode};

pub fn d(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

pub fn date(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).expect("date")
}

pub fn ts(n: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(1_790_000_000 + n, 0)
        .expect("timestamp")
        .fixed_offset()
}

pub fn snapshot(name: &str, ico: &str) -> serde_json::Value {
    json!({ "name": name, "ico": ico, "dic": format!("CZ{ico}"), "street": "Vzorová 1",
            "city": "Praha", "zip": "11000", "country": "CZ", "registration": null,
            "vatPayer": null })
}

pub fn doc() -> document::Model {
    document::Model {
        id: Uuid::new_v4(),
        space_id: Uuid::nil(),
        direction: Direction::Issued,
        doc_type: DocType::Invoice,
        status: Status::Issued,
        number: Some("2026000001".into()),
        number_year: Some(2026),
        number_seq: Some(1),
        imported: false,
        contact_id: None,
        issue_date: date(3, 1),
        tax_point_date: Some(date(3, 1)),
        due_date: Some(date(3, 15)),
        currency: "CZK".into(),
        exchange_rate: None,
        exchange_rate_date: None,
        exchange_rate_source: None,
        locale: "cs".into(),
        vat_mode: VatMode::Standard,
        bank_account_id: None,
        payment_method: PaymentMethod::BankTransfer,
        variable_symbol: Some("2026000001".into()),
        constant_symbol: None,
        order_ref: None,
        header_note: Some("Hlavička".into()),
        footer_note: None,
        internal_note: Some("Interní".into()),
        rounding_mode: RoundingMode::Unit,
        supplier_snapshot: Some(snapshot("Dodavatel s.r.o.", "44444443")),
        customer_snapshot: Some(snapshot("Odběratel a.s.", "12345679")),
        bank_snapshot: None,
        total_base: d("1000"),
        total_vat: d("210.40"),
        total: d("1210.40"),
        rounding: d("-0.40"),
        payable: d("1210"),
        total_czk: None,
        paid: Decimal::ZERO,
        sent_at: None,
        cancelled_at: None,
        cancel_reason: None,
        related_document_id: None,
        payment_id: None,
        correction_reason: None,
        pdf_path: None,
        pdf_sha256: None,
        pdf_rendered_at: None,
        pdf_size: None,
        supplier_number: Some("ignored".into()),
        received_date: Some(date(3, 2)),
        vat_deductible: true,
        supplier_account: None,
        category_id: None,
        custom_fields: json!({}),
        original_path: None,
        original_sha256: None,
        original_size: None,
        original_uploaded_at: None,
        unlocked_at: None,
        unlocked_by: None,
        unlock_reason: None,
        edited_since_unlock: false,
        created_at: ts(0),
        updated_at: ts(0),
    }
}

pub fn recap(doc: &document::Model, rate: &str, base: &str, vat: &str) -> vat_recap::Model {
    vat_recap::Model {
        document_id: doc.id,
        vat_rate: d(rate),
        base: d(base),
        vat: d(vat),
        base_czk: None,
        vat_czk: None,
    }
}
