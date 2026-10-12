//! The batched import lookups (`import::batch`, one query per kind for a
//! whole preview / confirm) answer exactly as the per-entry ones
//! (`import::lookup`) over a matrix of parties, numbers and references.

mod common;

use chrono::NaiveDate;
use common::csvio::{HEADER, confirm, doc_id, file, row};
use common::documents::{create_contact, set_company};
use common::{TestDb, router};
use invoice::document::compute::Totals;
use invoice::document::line::{PaymentMethod, VatMode};
use invoice::import::model::{ContactRule, IssuedBank, Party, Plan};
use invoice::import::{batch, lookup};
use invoice::settings::doc_type::{DocType, ISSUED, RECEIVED};
use rust_decimal::Decimal;
use serde_json::json;

fn party(name: &str, ico: Option<&str>, dic: Option<&str>) -> Party {
    Party {
        name: name.into(),
        ico: ico.map(Into::into),
        dic: dic.map(Into::into),
        country: "CZ".into(),
        ..Default::default()
    }
}

fn plan(
    direction: &'static str,
    doc_type: DocType,
    number: &str,
    p: &Party,
    rule: ContactRule,
    original_ref: Option<&str>,
) -> Plan {
    let issued = direction == ISSUED;
    Plan {
        direction,
        doc_type,
        number: number.into(),
        supplier: if issued {
            party("Naše firma", Some("44444443"), None)
        } else {
            p.clone()
        },
        customer: issued.then(|| p.clone()),
        vat_payer: None,
        issue_date: NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
        tax_point_date: None,
        due_date: None,
        received_date: None,
        currency: "CZK".into(),
        rate: None,
        vat_mode: VatMode::Standard,
        lines: Vec::new(),
        totals: Totals {
            recap: Vec::new(),
            base: Decimal::ZERO,
            vat: Decimal::ZERO,
            total: Decimal::ZERO,
            rounding: Decimal::ZERO,
            payable: Decimal::ZERO,
            total_czk: None,
        },
        gross: Decimal::ZERO,
        payment_method: PaymentMethod::BankTransfer,
        variable_symbol: None,
        constant_symbol: None,
        bank: None,
        note: None,
        locale: "cs".into(),
        original_ref: original_ref.map(Into::into),
        warnings: Vec::new(),
        contact_rule: rule,
        issued_bank: IssuedBank::CurrencyDefault,
    }
}

/// Stored: issued F-1 for 87654326, received R-1 from 12345679 and R-2
/// from "Beta s.r.o." (no IČO); contacts Alfa (DIČ only), Beta (name only),
/// Gama (IČO 12345679).
async fn seed(db: &TestDb) {
    let app = router(db.conn.clone());
    set_company(&app, true).await;
    create_contact(
        &app,
        json!({ "name": "Alfa s.r.o.", "ico": null, "dic": "CZ87654326" }),
    )
    .await;
    create_contact(&app, json!({ "name": "Beta s.r.o.", "ico": null })).await;
    create_contact(&app, json!({ "name": "Gama s.r.o.", "ico": "12345679" })).await;
    let doc = |direction: &str, number: (&str, &str), name: &str, ico: &str| {
        row(&[
            ("direction", direction),
            ("doc_type", "invoice"),
            ("number", number.0),
            ("supplier_number", number.1),
            ("issue_date", "1.3.2026"),
            ("counterparty_name", name),
            ("counterparty_ico", ico),
            ("base_0", "100"),
            ("total", "100"),
        ])
    };
    let rows = [
        doc("issued", ("F-1", ""), "Odběratel a.s.", "87654326"),
        doc("received", ("", "R-1"), "Gama s.r.o.", "12345679"),
        doc("received", ("", "R-2"), "Beta s.r.o.", ""),
    ];
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let bytes = file(HEADER, &refs);
    let results = confirm(&app, &bytes, &["row:2", "row:3", "row:4"]).await;
    for key in ["row:2", "row:3", "row:4"] {
        doc_id(&results, key);
    }
}

#[tokio::test]
async fn batched_lookups_match_the_per_entry_ones() {
    let db = TestDb::new().await;
    seed(&db).await;
    let parties = [
        party("X", Some("87654326"), None),
        party("Gama", Some("12345679"), None),
        party("Beta s.r.o.", None, None),
        party("  beta S.R.O. ", None, None),
        party("Jiné", None, Some("CZ87654326")),
        party("Alfa s.r.o.", Some("11111111"), Some("CZ87654326")),
        party("Alfa s.r.o.", None, Some("CZ00000000")),
        party("   ", None, None),
    ];
    let mut plans = Vec::new();
    for p in &parties {
        for direction in [ISSUED, RECEIVED] {
            for rule in [ContactRule::IcoOrName, ContactRule::IcoDicName] {
                for number in ["F-1", "R-1", "R-2", "none"] {
                    plans.push(plan(direction, DocType::Invoice, number, p, rule, None));
                    plans.push(plan(
                        direction,
                        DocType::CreditNote,
                        "D-1",
                        p,
                        rule,
                        Some(number),
                    ));
                }
                // No link target for a proforma.
                plans.push(plan(
                    direction,
                    DocType::Proforma,
                    "P",
                    p,
                    rule,
                    Some("F-1"),
                ));
            }
        }
    }
    let refs: Vec<&Plan> = plans.iter().collect();
    let (conn, space) = (&db.conn, db.space);
    let duplicates = batch::duplicates(conn, space, &refs).await.expect("dup");
    let contacts = batch::contacts(conn, space, &refs).await.expect("contacts");
    let related = batch::related(conn, space, &refs).await.expect("related");
    for (n, p) in plans.iter().enumerate() {
        let what = format!("{n}: {} {} {:?}", p.direction, p.number, p.counterparty());
        let dup = lookup::duplicate(conn, space, p).await.expect("dup");
        assert_eq!(duplicates.contains(&n), dup, "duplicate {what}");
        let contact = match p.counterparty() {
            Some(c) => lookup::contact(conn, space, c, p.contact_rule)
                .await
                .expect("contact")
                .is_some(),
            None => false,
        };
        assert_eq!(contacts.contains(&n), contact, "contact {what}");
        let rel = lookup::related(conn, space, p).await.expect("related");
        assert_eq!(related.contains(&n), rel.is_some(), "related {what}");
    }
    // The matrix hits both answers of every kind.
    for (kind, set) in [
        ("duplicates", &duplicates),
        ("contacts", &contacts),
        ("related", &related),
    ] {
        assert!(!set.is_empty() && set.len() < plans.len(), "{kind}");
    }
    assert!(
        batch::duplicates(conn, space, &[])
            .await
            .expect("empty")
            .is_empty()
    );
}
