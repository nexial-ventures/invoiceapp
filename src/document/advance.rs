//! Pure validation of `advance` lines against the documents they deduct, and
//! filling in their server-generated description and deducted amounts.

use std::collections::HashMap;

use rust_decimal::Decimal;
use uuid::Uuid;

use super::line::{AdvanceRow, LineData, Status, VatMode};
use crate::document::doc_type::DocType;
use crate::error::FieldErrors;

/// A document an advance line may reference, as loaded from the database.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvanceSource {
    pub id: Uuid,
    pub doc_type: DocType,
    pub status: Status,
    pub contact_id: Option<Uuid>,
    pub currency: String,
    pub number: Option<String>,
    /// For a proforma: whether its payments issue DDPPs (then the proforma
    /// itself can never be deducted directly).
    pub issues_ddpp: bool,
    pub paid: Decimal,
    /// Stored recap (a DDPP's amounts are what gets deducted).
    pub recap: Vec<AdvanceRow>,
    /// Non-cancelled invoices whose lines reference this document.
    pub referenced_by: Vec<Uuid>,
}

/// The deducting document as far as advance rules care.
pub struct AdvanceCtx<'a> {
    pub doc_type: DocType,
    /// A DDPP already taxed the advance: only a `standard` invoice may
    /// deduct its VAT.
    pub vat_mode: VatMode,
    /// `None` while creating.
    pub document_id: Option<Uuid>,
    pub related_document_id: Option<Uuid>,
    pub contact_id: Option<Uuid>,
    pub currency: &'a str,
    pub locale: &'a str,
    pub sources: &'a HashMap<Uuid, AdvanceSource>,
}

/// Whether payments of a proforma issued with this supplier VAT status and
/// mode create DDPPs.
pub fn issues_ddpp(supplier_vat_payer: Option<bool>, vat_mode: VatMode) -> bool {
    supplier_vat_payer == Some(true) && vat_mode == VatMode::Standard
}

pub fn description(locale: &str, number: &str) -> String {
    match locale {
        "en" => format!("Advance deduction {number}"),
        _ => format!("Odpočet zálohy {number}"),
    }
}

/// Every advance line's referenced document ids (for loading the sources).
pub fn referenced_ids(lines: &[LineData]) -> Vec<Uuid> {
    lines
        .iter()
        .filter_map(|l| match l {
            LineData::Advance(a) => Some(a.document_id),
            _ => None,
        })
        .collect()
}

/// Validate every advance line and fill its description and recap. Errors
/// are `lines.N.advanceDocumentId` (`invalid` / `duplicate`), or
/// `lines.N.kind: invalid` on a document type that cannot hold advances.
pub fn resolve(lines: &mut [LineData], ctx: &AdvanceCtx) -> FieldErrors {
    let mut e = FieldErrors::new();
    let mut seen: Vec<Uuid> = Vec::new();
    for (i, line) in lines.iter_mut().enumerate() {
        let LineData::Advance(a) = line else { continue };
        if ctx.doc_type != DocType::Invoice {
            e.add(&format!("lines.{i}.kind"), "invalid");
            continue;
        }
        let field = format!("lines.{i}.advanceDocumentId");
        let Some(src) = ctx.sources.get(&a.document_id) else {
            e.add(&field, "invalid");
            continue;
        };
        let recap = match deducted(src, ctx) {
            Some(r) => r,
            None => {
                e.add(&field, "invalid");
                continue;
            }
        };
        let taken_elsewhere = src
            .referenced_by
            .iter()
            .any(|d| Some(*d) != ctx.document_id);
        if seen.contains(&src.id) || taken_elsewhere {
            e.add(&field, "duplicate");
            continue;
        }
        seen.push(src.id);
        a.description = description(ctx.locale, src.number.as_deref().unwrap_or_default());
        a.recap = recap;
    }
    e
}

/// What `src` deducts from the document in `ctx`, or `None` if it may not.
fn deducted(src: &AdvanceSource, ctx: &AdvanceCtx) -> Option<Vec<AdvanceRow>> {
    if src.status != Status::Issued || src.currency != ctx.currency {
        return None;
    }
    if src.doc_type == DocType::AdvanceTaxDoc {
        let same_contact = ctx.contact_id.is_some() && src.contact_id == ctx.contact_id;
        // A fully corrected DDPP has nothing left to deduct.
        let ok = same_contact && ctx.vat_mode == VatMode::Standard && !src.recap.is_empty();
        return ok.then(|| src.recap.clone());
    }
    // Non-payer form: only the proforma this invoice settles, and only one
    // whose payments did not already produce DDPPs.
    let own_proforma = src.doc_type == DocType::Proforma
        && ctx.related_document_id == Some(src.id)
        && !src.issues_ddpp;
    own_proforma.then(|| {
        vec![AdvanceRow {
            vat_rate: Decimal::ZERO,
            base: src.paid,
            vat: Decimal::ZERO,
            base_czk: None,
            vat_czk: None,
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::line::AdvanceData;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    fn ddpp(id: u128, contact: u128) -> AdvanceSource {
        AdvanceSource {
            id: Uuid::from_u128(id),
            doc_type: DocType::AdvanceTaxDoc,
            status: Status::Issued,
            contact_id: Some(Uuid::from_u128(contact)),
            currency: "CZK".into(),
            number: Some(format!("DP2026000{id}")),
            issues_ddpp: false,
            paid: Decimal::ZERO,
            recap: vec![AdvanceRow {
                vat_rate: d("21"),
                base: d("100"),
                vat: d("21"),
                base_czk: None,
                vat_czk: None,
            }],
            referenced_by: vec![],
        }
    }

    fn adv(id: u128) -> LineData {
        LineData::Advance(AdvanceData {
            document_id: Uuid::from_u128(id),
            description: String::new(),
            recap: vec![],
        })
    }

    fn ctx<'a>(sources: &'a HashMap<Uuid, AdvanceSource>, related: Option<u128>) -> AdvanceCtx<'a> {
        AdvanceCtx {
            doc_type: DocType::Invoice,
            vat_mode: VatMode::Standard,
            document_id: Some(Uuid::from_u128(99)),
            related_document_id: related.map(Uuid::from_u128),
            contact_id: Some(Uuid::from_u128(7)),
            currency: "CZK",
            locale: "cs",
            sources,
        }
    }

    fn errors(pairs: &[(&str, &'static str)]) -> FieldErrors {
        let mut e = FieldErrors::new();
        for (f, r) in pairs {
            e.add(f, r);
        }
        e
    }

    #[test]
    fn fills_ddpp_deduction() {
        let sources = HashMap::from([(Uuid::from_u128(1), ddpp(1, 7))]);
        let mut lines = vec![adv(1)];
        assert!(resolve(&mut lines, &ctx(&sources, None)).is_empty());
        let LineData::Advance(a) = &lines[0] else {
            panic!("advance expected")
        };
        assert_eq!(a.description, "Odpočet zálohy DP20260001");
        assert_eq!(a.recap, sources[&Uuid::from_u128(1)].recap);
    }

    #[test]
    fn rejects_foreign_contact_currency_state_and_duplicates() {
        let mut cancelled = ddpp(3, 7);
        cancelled.status = Status::Cancelled;
        let mut eur = ddpp(4, 7);
        eur.currency = "EUR".into();
        let mut taken = ddpp(5, 7);
        taken.referenced_by = vec![Uuid::from_u128(50)];
        let mut mine = ddpp(6, 7);
        mine.referenced_by = vec![Uuid::from_u128(99)];
        let sources: HashMap<_, _> = [ddpp(1, 7), ddpp(2, 8), cancelled, eur, taken, mine]
            .into_iter()
            .map(|s| (s.id, s))
            .collect();
        let mut lines = vec![
            adv(1),
            adv(1),
            adv(2),
            adv(3),
            adv(4),
            adv(5),
            adv(6),
            adv(77),
        ];
        let e = resolve(&mut lines, &ctx(&sources, None));
        assert_eq!(
            e,
            errors(&[
                ("lines.1.advanceDocumentId", "duplicate"),
                ("lines.2.advanceDocumentId", "invalid"),
                ("lines.3.advanceDocumentId", "invalid"),
                ("lines.4.advanceDocumentId", "invalid"),
                ("lines.5.advanceDocumentId", "duplicate"),
                ("lines.7.advanceDocumentId", "invalid"),
            ])
        );
    }

    #[test]
    fn ddpp_needs_a_standard_invoice_but_the_non_payer_form_does_not() {
        let mut proforma = ddpp(10, 7);
        proforma.doc_type = DocType::Proforma;
        proforma.paid = d("100");
        let sources: HashMap<_, _> = [ddpp(1, 7), proforma]
            .into_iter()
            .map(|s| (s.id, s))
            .collect();
        for mode in [VatMode::ReverseCharge, VatMode::Exempt, VatMode::NonPayer] {
            let mut c = ctx(&sources, Some(10));
            c.vat_mode = mode;
            let e = resolve(&mut [adv(1), adv(10)], &c);
            assert_eq!(
                e,
                errors(&[("lines.0.advanceDocumentId", "invalid")]),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn non_payer_form_deducts_paid_of_own_proforma_only() {
        let mut proforma = ddpp(10, 7);
        proforma.doc_type = DocType::Proforma;
        proforma.paid = d("500");
        let mut payer = proforma.clone();
        payer.id = Uuid::from_u128(11);
        payer.issues_ddpp = true;
        let sources: HashMap<_, _> = [proforma, payer].into_iter().map(|s| (s.id, s)).collect();

        let mut lines = vec![adv(10)];
        assert!(resolve(&mut lines, &ctx(&sources, Some(10))).is_empty());
        let LineData::Advance(a) = &lines[0] else {
            panic!("advance expected")
        };
        assert_eq!((a.recap[0].vat_rate, a.recap[0].base), (d("0"), d("500")));

        let e = resolve(&mut [adv(10)], &ctx(&sources, None));
        assert_eq!(e, errors(&[("lines.0.advanceDocumentId", "invalid")]));
        let e = resolve(&mut [adv(11)], &ctx(&sources, Some(11)));
        assert_eq!(e, errors(&[("lines.0.advanceDocumentId", "invalid")]));
    }

    #[test]
    fn fully_corrected_ddpp_is_invalid() {
        let mut done = ddpp(1, 7);
        done.recap.clear();
        let sources = HashMap::from([(Uuid::from_u128(1), done)]);
        let e = resolve(&mut [adv(1)], &ctx(&sources, None));
        assert_eq!(e, errors(&[("lines.0.advanceDocumentId", "invalid")]));
    }

    #[test]
    fn only_invoices_hold_advances() {
        let sources = HashMap::from([(Uuid::from_u128(1), ddpp(1, 7))]);
        let mut c = ctx(&sources, None);
        c.doc_type = DocType::Proforma;
        let e = resolve(&mut [adv(1)], &c);
        assert_eq!(e, errors(&[("lines.0.kind", "invalid")]));
        assert_eq!(description("en", "DP1"), "Advance deduction DP1");
        assert!(issues_ddpp(Some(true), VatMode::Standard));
        assert!(!issues_ddpp(Some(true), VatMode::ReverseCharge));
        assert!(!issues_ddpp(Some(false), VatMode::Standard));
    }
}
