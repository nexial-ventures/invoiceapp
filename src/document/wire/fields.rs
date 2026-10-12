//! Field validators shared by the issued and received document bodies.

use rust_decimal::Decimal;

use crate::document::line::{MAX_RATE, VatMode};
use crate::document::model::Existing;
use crate::num_parse::decimal;
use crate::settings::doc_type::DocType;
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

/// The document type a request may set: on create an invoice, proforma or
/// simplified document (an imported document: any of the seven); on update
/// only the draft's own type.
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
        (None, None) => Ok(DocType::Invoice),
        (None, Some(Ok(t))) if imported => Ok(t),
        (None, Some(Ok(t @ (DocType::Invoice | DocType::Proforma | DocType::Simplified)))) => Ok(t),
        _ => Err("invalid"),
    }
}
