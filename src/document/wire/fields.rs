//! Field validators shared by the issued and received document bodies.

use rust_decimal::Decimal;

use crate::document::doc_type::DocType;
use crate::document::line::{MAX_RATE, VatMode};
use crate::document::model::Existing;
use crate::num_parse::decimal;
use crate::validation::{self as v, Check};

/// Digits only, at most `max` (variable / constant symbol).
pub fn digits(s: Option<&str>, max: usize) -> Check<Option<String>> {
    let s = v::opt_text(s, max)?;
    match s {
        Some(d) if !d.bytes().all(|c| c.is_ascii_digit()) => Err("invalid"),
        other => Ok(other),
    }
}

pub fn exchange_rate(s: Option<&str>) -> Check<Option<Decimal>> {
    match s.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => {
            let r = decimal(s, 6, MAX_RATE)?;
            if r <= Decimal::ZERO {
                Err("invalid")
            } else {
                Ok(Some(r))
            }
        }
    }
}

pub fn parse_vat_mode(s: &str) -> Check<VatMode> {
    VatMode::parse(s.trim()).ok_or("invalid")
}

/// The types a user creates directly (no original to correct or payment to
/// document).
fn standalone(t: DocType) -> bool {
    matches!(
        t,
        DocType::Invoice | DocType::Proforma | DocType::Simplified
    )
}

/// The document type a request may set: on create an invoice, proforma or
/// simplified document (an imported document: any of the seven); on update
/// the draft's own type — or, for an unlocked draft of one of those three,
/// another of them.
pub fn doc_type(
    requested: Option<&str>,
    existing: Option<&Existing>,
    imported: bool,
) -> Check<DocType> {
    let parsed = requested.map(|t| DocType::parse_document(t.trim()).ok_or("invalid"));
    match (existing, parsed) {
        (_, Some(Err(r))) => Err(r),
        (Some(x), None) => Ok(x.doc_type),
        (Some(x), Some(Ok(t))) if t == x.doc_type => Ok(t),
        (Some(x), Some(Ok(t))) if x.unlocked && standalone(x.doc_type) && standalone(t) => Ok(t),
        (None, None) => Ok(DocType::Invoice),
        (None, Some(Ok(t))) if imported => Ok(t),
        (None, Some(Ok(t))) if standalone(t) => Ok(t),
        _ => Err("invalid"),
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn draft(doc_type: DocType, unlocked: bool) -> Existing {
        Existing {
            id: Uuid::from_u128(1),
            doc_type,
            imported: false,
            related_document_id: None,
            contact_id: None,
            currency: "CZK".into(),
            locale: "cs".into(),
            vat_mode: VatMode::Standard,
            exchange_rate: None,
            number: unlocked.then(|| "20260001".into()),
            unlocked,
        }
    }

    #[test]
    fn only_an_unlocked_standalone_draft_changes_type() {
        let fresh = draft(DocType::Invoice, false);
        assert_eq!(
            doc_type(Some("proforma"), Some(&fresh), false),
            Err("invalid")
        );
        let unlocked = draft(DocType::Invoice, true);
        for t in ["proforma", "simplified", "invoice"] {
            assert!(doc_type(Some(t), Some(&unlocked), false).is_ok(), "{t}");
        }
        assert_eq!(
            doc_type(Some("credit_note"), Some(&unlocked), false),
            Err("invalid")
        );
        let credit = draft(DocType::CreditNote, true);
        assert_eq!(
            doc_type(Some("invoice"), Some(&credit), false),
            Err("invalid")
        );
        assert_eq!(
            doc_type(None, Some(&credit), false),
            Ok(DocType::CreditNote)
        );
    }
}
