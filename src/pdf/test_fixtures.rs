//! Shared fixtures of the payload unit tests.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::format::Locale;
use super::payload::*;
use crate::document::line::{Status, VatMode};
use crate::document::snapshot::Recap as DtoRecap;
use crate::document::snapshot::{BankSnapshot, PartySnapshot, Totals};
use crate::document::wire::dto::Line;
use crate::document::wire::line_out::{ItemLine, SubtotalLine, TextLine};

pub const NB: char = '\u{a0}';

pub fn d(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

pub fn day(m: u32, dd: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, dd).expect("date")
}

pub fn item(position: i32, price: &str, discount: &str, rate: &str, base: &str) -> Line {
    Line::Item(ItemLine {
        position,
        description: format!("Item {position}"),
        quantity: d("2"),
        unit: Some("ks".into()),
        unit_price: d(price),
        discount_pct: d(discount),
        vat_rate: d(rate),
        base: d(base),
    })
}

pub fn subtotal(position: i32, refs: Vec<i32>, collapse: bool, base: &str) -> Line {
    Line::Subtotal(SubtotalLine {
        position,
        description: format!("Group {position}"),
        refs,
        collapse,
        base: d(base),
        vat_rate: d("21"),
    })
}

pub fn totals(base: &str, vat: &str, rounding: &str, payable: &str) -> Totals {
    Totals {
        recap: vec![DtoRecap {
            vat_rate: d("21"),
            base: d(base),
            vat: d(vat),
            base_czk: None,
            vat_czk: None,
        }],
        base: d(base),
        vat: d(vat),
        total: d(base) + d(vat),
        rounding: d(rounding),
        payable: d(payable),
        total_czk: None,
    }
}

pub fn party(name: &str, country: &str) -> PartySnapshot {
    PartySnapshot {
        name: name.into(),
        ico: Some("44444443".into()),
        dic: None,
        street: "Hlavní 1".into(),
        city: "Praha".into(),
        zip: "110 00".into(),
        country: country.into(),
        registration: Some("C 1 u MS v Praze".into()),
        vat_payer: Some(true),
        email: Some("a@example.com".into()),
        phone: Some(" ".into()),
        web: None,
    }
}

pub fn bank() -> BankSnapshot {
    BankSnapshot {
        account_number: Some("19-2000145399/0800".into()),
        iban: Some("CZ6508000000192000145399".into()),
        bic: Some("GIBACZPX".into()),
    }
}

pub struct Fixture {
    pub supplier: PartySnapshot,
    pub customer: PartySnapshot,
    pub bank: BankSnapshot,
    pub lines: Vec<Line>,
    pub totals: Totals,
}

impl Fixture {
    pub fn new(lines: Vec<Line>, totals: Totals) -> Self {
        Self {
            supplier: party("Dodavatel s.r.o.", "CZ"),
            customer: party("Kunde GmbH", "DE"),
            bank: bank(),
            lines,
            totals,
        }
    }

    pub fn input(&self) -> Input<'_> {
        Input {
            locale: Locale::Cs,
            status: Status::Issued,
            doc_type: "invoice",
            vat_mode: VatMode::Standard,
            number: Some("20260001"),
            issue_date: day(10, 8),
            tax_point_date: Some(day(10, 8)),
            due_date: day(10, 22),
            currency: "CZK",
            rate: None,
            payment_method: "bank_transfer",
            variable_symbol: Some("20260001"),
            constant_symbol: None,
            order_ref: Some(" "),
            header_note: Some("Hlavička"),
            footer_note: None,
            correction_reason: None,
            parent_number: None,
            parent_doc_type: None,
            supplier: Some(&self.supplier),
            customer: Some(&self.customer),
            bank: Some(&self.bank),
            lines: &self.lines,
            totals: &self.totals,
        }
    }
}

pub fn standard() -> Fixture {
    Fixture::new(
        vec![
            item(1, "100", "10", "21", "180"),
            item(2, "50", "0", "21", "100"),
            item(3, "25", "0", "21", "50"),
            subtotal(4, vec![2, 3], true, "150"),
            Line::Text(TextLine {
                position: 5,
                description: "Díky".into(),
            }),
            subtotal(6, vec![1], false, "180"),
        ],
        totals("330", "69.30", "0.70", "400"),
    )
}

pub fn values(rows: &[TotalRow]) -> Vec<(&str, &str, bool)> {
    rows.iter()
        .map(|r| (r.label.as_str(), r.value.as_str(), r.strong))
        .collect()
}
