//! Manual import of issued documents: the `imported` flag, the document's
//! own `number` and its informational `relatedDocumentId`.

use uuid::Uuid;

use super::input::DocumentInput;
use crate::document::meta::check_related;
use crate::document::model::{Context, Existing};
use crate::error::FieldErrors;
use crate::settings::doc_type::{DocType, ISSUED};
use crate::validation as v;

pub const MAX_NUMBER: usize = 40;

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
/// checked, no `advance` lines. Native: no `number`; the link is the one the
/// server set (settlement / credit note) — a request value is ignored.
pub fn fields(
    imported: bool,
    doc_type: DocType,
    input: &DocumentInput,
    ctx: &Context,
    e: &mut FieldErrors,
) -> (Option<String>, Option<Uuid>) {
    let existing = ctx.existing.as_ref();
    let number = input.number.as_deref();
    if !imported {
        if v::opt_text(number, MAX_NUMBER).ok().flatten().is_some() {
            e.add("number", "invalid");
        }
        return (None, existing.and_then(|x| x.related_document_id));
    }
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
        ISSUED,
        self_id,
        ctx.related.as_ref(),
        e,
    );
    (number, related)
}
