//! Document metadata shared by issued and received documents: category,
//! custom fields, internal note — and the informational `relatedDocumentId`
//! of imported / received documents.

use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::document::custom_fields::{self, FieldDef, Values};
use crate::document::entity::document;
use crate::error::FieldErrors;
use crate::settings::doc_type::{DocType, RECEIVED};
use crate::settings::entity::category;
use crate::validation as v;

/// What metadata validation needs from the database.
#[derive(Debug, Clone, Default)]
pub struct MetaCtx {
    /// The category named by the request, if it exists.
    pub category: Option<category::Model>,
    /// Active definitions applying to the document's direction.
    pub defs: Vec<FieldDef>,
    /// The saved document's values (`PUT` / issue / metadata).
    pub stored_category: Option<Uuid>,
    pub stored_fields: Values,
}

/// `PUT /api/documents/{id}/metadata` body (replaces all three).
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct MetadataInput {
    pub category_id: Option<Uuid>,
    #[schema(value_type = Object)]
    pub custom_fields: Option<Values>,
    pub internal_note: Option<String>,
}

/// Validated metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub category_id: Option<Uuid>,
    pub custom_fields: Values,
}

/// Category kind a direction accepts.
pub fn category_kind(direction: &str) -> &'static str {
    if direction == RECEIVED {
        "expense"
    } else {
        "income"
    }
}

/// `categoryId`: must exist with the direction's kind (`invalid`); an
/// inactive one only stays where it already is (`inactive`).
pub fn validate(
    category_id: Option<Uuid>,
    fields: Option<&Values>,
    direction: &str,
    ctx: &MetaCtx,
    e: &mut FieldErrors,
) -> Meta {
    let category_id = category_id.filter(|id| {
        let check = match &ctx.category {
            Some(c) if c.id != *id || c.kind != category_kind(direction) => Err("invalid"),
            Some(c) if !c.active && ctx.stored_category != Some(c.id) => Err("inactive"),
            Some(_) => Ok(()),
            None => Err("invalid"),
        };
        e.check("categoryId", check).is_some()
    });
    let empty = Values::new();
    let custom_fields =
        custom_fields::validate(fields.unwrap_or(&empty), &ctx.defs, &ctx.stored_fields, e);
    Meta {
        category_id,
        custom_fields,
    }
}

impl MetadataInput {
    pub fn validate(
        self,
        direction: &str,
        ctx: &MetaCtx,
    ) -> Result<(Meta, Option<String>), crate::error::AppError> {
        let mut e = FieldErrors::new();
        let meta = validate(
            self.category_id,
            self.custom_fields.as_ref(),
            direction,
            ctx,
            &mut e,
        );
        let note = e
            .check(
                "internalNote",
                v::opt_text(self.internal_note.as_deref(), 2000),
            )
            .flatten();
        e.into_result()?;
        Ok((meta, note))
    }
}

/// The document types a document of `doc_type` may link to: a DDPP / final
/// invoice → proforma, a credit / debit note → invoice or simplified
/// document, a DDPP correction → DDPP; a proforma or simplified document
/// links nowhere.
pub fn related_types(doc_type: DocType) -> &'static [DocType] {
    match doc_type {
        DocType::Invoice | DocType::AdvanceTaxDoc => &[DocType::Proforma],
        DocType::CreditNote | DocType::DebitNote => &[DocType::Invoice, DocType::Simplified],
        DocType::AdvanceCreditNote => &[DocType::AdvanceTaxDoc],
        _ => &[],
    }
}

/// `relatedDocumentId` of an imported / received document: the target must
/// be a non-draft document of the same direction and the right type.
pub fn check_related(
    requested: Option<Uuid>,
    doc_type: DocType,
    direction: &str,
    self_id: Option<Uuid>,
    target: Option<&document::Model>,
    e: &mut FieldErrors,
) -> Option<Uuid> {
    let id = requested?;
    let ok = target.is_some_and(|t| {
        t.id == id
            && Some(id) != self_id
            && t.direction == direction
            && t.status != "draft"
            && related_types(doc_type)
                .iter()
                .any(|r| r.as_str() == t.doc_type)
    });
    if ok {
        Some(id)
    } else {
        e.add("relatedDocumentId", "invalid");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::custom_fields::FieldType;

    fn cat(kind: &str, active: bool) -> category::Model {
        let now = chrono::Utc::now().into();
        category::Model {
            id: Uuid::from_u128(7),
            space_id: Uuid::nil(),
            name: "Software".into(),
            kind: kind.into(),
            active,
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }

    fn run(id: Option<Uuid>, dir: &str, ctx: &MetaCtx) -> (Meta, FieldErrors) {
        let mut e = FieldErrors::new();
        (validate(id, None, dir, ctx, &mut e), e)
    }

    #[test]
    fn category_rules() {
        let id = Some(Uuid::from_u128(7));
        let ctx = |c| MetaCtx {
            category: Some(c),
            ..Default::default()
        };
        let (m, e) = run(id, "received", &ctx(cat("expense", true)));
        assert!(e.is_empty());
        assert_eq!(m.category_id, id);
        let (_, e) = run(id, "issued", &ctx(cat("expense", true)));
        assert_eq!(e.get("categoryId"), Some("invalid"));
        let (_, e) = run(id, "issued", &ctx(cat("income", false)));
        assert_eq!(e.get("categoryId"), Some("inactive"));
        let mut kept = ctx(cat("income", false));
        kept.stored_category = id;
        assert!(run(id, "issued", &kept).1.is_empty());
        let (_, e) = run(id, "issued", &MetaCtx::default());
        assert_eq!(e.get("categoryId"), Some("invalid"));
        assert!(run(None, "issued", &MetaCtx::default()).1.is_empty());
    }

    #[test]
    fn custom_fields_are_validated() {
        let ctx = MetaCtx {
            defs: vec![FieldDef {
                key: "project".into(),
                field_type: FieldType::Text,
                options: vec![],
                required: true,
            }],
            ..Default::default()
        };
        let (_, e) = run(None, "issued", &ctx);
        assert_eq!(e.get("customFields.project"), Some("required"));
    }

    #[test]
    fn related_type_table() {
        use DocType::*;
        assert_eq!(related_types(Invoice), [Proforma]);
        assert_eq!(related_types(AdvanceTaxDoc), [Proforma]);
        assert_eq!(related_types(CreditNote), [Invoice, Simplified]);
        assert_eq!(related_types(DebitNote), [Invoice, Simplified]);
        assert_eq!(related_types(AdvanceCreditNote), [AdvanceTaxDoc]);
        assert!(related_types(Proforma).is_empty());
        assert!(related_types(Simplified).is_empty());
    }
}
