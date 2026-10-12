//! The printed `reference` line: what a correction corrects (+ its reason),
//! and the proforma of a DDPP. Pure.

use super::format::Locale;
use crate::document::doc_type::DocType;

/// `doc_type` refers to its parent (`parent_type`, `parent` number); `None`
/// for documents that print no reference.
pub fn reference(
    doc_type: &str,
    parent_type: Option<&str>,
    parent: &str,
    reason: Option<&str>,
    locale: Locale,
) -> Option<String> {
    let cs = locale == Locale::Cs;
    let simplified = parent_type == Some(DocType::Simplified.as_str());
    let doc_type = DocType::parse(doc_type);
    let head = match doc_type {
        Some(DocType::AdvanceTaxDoc) if cs => return Some(format!("K zálohové faktuře {parent}")),
        Some(DocType::AdvanceTaxDoc) => return Some(format!("For proforma invoice {parent}")),
        Some(DocType::CreditNote) if cs => "Opravný daňový doklad",
        Some(DocType::CreditNote) => "Credit note",
        Some(DocType::DebitNote) if cs => "Opravný daňový doklad – vrubopis",
        Some(DocType::DebitNote) => "Debit note",
        Some(DocType::AdvanceCreditNote) if cs => {
            "Opravný daňový doklad k daňovému dokladu k přijaté platbě"
        }
        Some(DocType::AdvanceCreditNote) => "Correction of advance payment tax document",
        _ => return None,
    };
    let head = match (doc_type, simplified, cs) {
        (Some(DocType::AdvanceCreditNote), _, _) => format!("{head} {parent}"),
        (_, true, true) => format!("{head} k zjednodušenému daňovému dokladu {parent}"),
        (_, true, false) => format!("{head} to simplified tax document {parent}"),
        (_, false, true) => format!("{head} k faktuře {parent}"),
        (_, false, false) => format!("{head} for invoice {parent}"),
    };
    Some(match reason {
        Some(r) if cs => format!("{head}\nDůvod opravy: {r}"),
        Some(r) => format!("{head}\nReason: {r}"),
        None => head,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CS: Locale = Locale::Cs;
    const EN: Locale = Locale::En;

    #[test]
    fn credit_and_debit_notes() {
        let inv = Some("invoice");
        assert_eq!(
            reference("credit_note", inv, "20260001", Some("Sleva"), CS).as_deref(),
            Some("Opravný daňový doklad k faktuře 20260001\nDůvod opravy: Sleva")
        );
        assert_eq!(
            reference("credit_note", inv, "20260001", None, EN).as_deref(),
            Some("Credit note for invoice 20260001")
        );
        assert_eq!(
            reference("debit_note", inv, "1", Some("Doúčtování"), CS).as_deref(),
            Some("Opravný daňový doklad – vrubopis k faktuře 1\nDůvod opravy: Doúčtování")
        );
        let s = Some("simplified");
        assert_eq!(
            reference("credit_note", s, "ZD1", None, CS).as_deref(),
            Some("Opravný daňový doklad k zjednodušenému daňovému dokladu ZD1")
        );
        assert_eq!(
            reference("debit_note", s, "ZD1", Some("x"), EN).as_deref(),
            Some("Debit note to simplified tax document ZD1\nReason: x")
        );
    }

    #[test]
    fn advances() {
        assert_eq!(
            reference("advance_tax_doc", Some("proforma"), "Z20260003", None, CS).as_deref(),
            Some("K zálohové faktuře Z20260003")
        );
        let ddpp = Some("advance_tax_doc");
        assert_eq!(
            reference("advance_credit_note", ddpp, "DP1", Some("Vráceno"), CS).as_deref(),
            Some(
                "Opravný daňový doklad k daňovému dokladu k přijaté platbě DP1\nDůvod opravy: Vráceno"
            )
        );
        assert_eq!(
            reference("advance_credit_note", ddpp, "DP1", None, EN).as_deref(),
            Some("Correction of advance payment tax document DP1")
        );
        assert_eq!(reference("invoice", Some("proforma"), "Z1", None, CS), None);
        assert_eq!(reference("simplified", None, "X", None, CS), None);
    }
}
