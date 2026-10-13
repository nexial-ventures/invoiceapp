//! Manual import of issued documents: the `imported` flag, the document's
//! own `number` and its informational `relatedDocumentId`.

use uuid::Uuid;

use super::input::DocumentInput;
use crate::document::doc_type::DocType;
use crate::document::line::Direction;
use crate::document::meta::check_related;
use crate::document::model::{Context, Existing};
use crate::error::FieldErrors;
use crate::validation as v;

pub const MAX_NUMBER: usize = 40;
/// Longest manual number of a native draft (a pattern of 40 characters
/// with a code and an overflowing sequence still fits).
pub const MAX_MANUAL_NUMBER: usize = 50;

/// Set on create (default `false`) and immutable: a `PUT` that sends a
/// different value → `imported: invalid` (omitted keeps it).
pub fn flag(requested: Option<bool>, existing: Option<&Existing>, e: &mut FieldErrors) -> bool {
    match (existing, requested) {
        (Some(x), Some(r)) if r != x.imported => {
            e.add("imported", "invalid");
            x.imported
        }
        (Some(x), _) => x.imported,
        (None, r) => r.unwrap_or(false),
    }
}

/// `(number, relatedDocumentId)`. Imported: `number` required, the link
/// checked, no `advance` lines. Native: an optional manual `number` (empty /
/// `null` = none, allocate at issue; omitted = keep the draft's number);
/// the link is the one the server set (settlement / credit note) — a
/// request value is ignored.
pub fn fields(
    imported: bool,
    doc_type: DocType,
    input: &DocumentInput,
    ctx: &Context,
    e: &mut FieldErrors,
) -> (Option<String>, Option<Uuid>) {
    let existing = ctx.existing.as_ref();
    if !imported {
        let number = match &input.number {
            // Omitted: the draft keeps what it has (nothing on create).
            None => existing.and_then(|x| x.number.clone()),
            Some(n) => e.check("number", manual_number(n.as_deref())).flatten(),
        };
        return (number, existing.and_then(|x| x.related_document_id));
    }
    let number = input.number.clone().flatten();
    let number = number.as_deref();
    for (i, l) in input.lines.iter().enumerate() {
        if l.kind == "advance" {
            e.add(&format!("lines.{i}.kind"), "invalid");
        }
    }
    let number = e.check(
        "number",
        v::required_text(number.unwrap_or_default(), MAX_NUMBER),
    );
    let self_id = existing.map(|x| x.id);
    let related = check_related(
        input.related_document_id,
        doc_type,
        Direction::Issued,
        self_id,
        ctx.related.as_ref(),
        e,
    );
    (number, related)
}

/// Trimmed; empty → `None`; else `[A-Za-z0-9/_-]`, at most
/// [`MAX_MANUAL_NUMBER`] characters (the characters a pattern can render).
pub fn manual_number(s: Option<&str>) -> v::Check<Option<String>> {
    match s.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(n)
            if n.len() <= MAX_MANUAL_NUMBER
                && n.chars().all(crate::settings::pattern::is_number_char) =>
        {
            Ok(Some(n.to_string()))
        }
        Some(_) => Err("invalid"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_numbers() {
        assert_eq!(manual_number(None), Ok(None));
        assert_eq!(manual_number(Some("  ")), Ok(None));
        assert_eq!(
            manual_number(Some(" ACME-2026/01_a ")),
            Ok(Some("ACME-2026/01_a".into()))
        );
        let longest = "1".repeat(MAX_MANUAL_NUMBER);
        assert_eq!(manual_number(Some(&longest)), Ok(Some(longest.clone())));
        for bad in [
            format!("{longest}1"),
            "A B".into(),
            "FA.1".into(),
            "Č1".into(),
        ] {
            assert_eq!(manual_number(Some(&bad)), Err("invalid"), "{bad}");
        }
    }
}
