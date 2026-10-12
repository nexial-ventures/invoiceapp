//! Database lookups of the import: existing contact, duplicate, related
//! original. [`super::check`] runs them for the preview and for the selected
//! entries of a confirm; [`super::store`] runs them again inside each entry's
//! transaction, the duplicate check under an advisory lock on the
//! document's identity.

use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use super::model::{ContactRule, Party, Plan};
use crate::contact::entity::contact;
use crate::document::entity::document::{Column, Entity};
use crate::document::line::Status;
use crate::document::meta::related_types;
use crate::error::AppError;
use crate::settings::doc_type::{DocType, ISSUED};
use crate::space::SpaceId;

/// Identity of a party: its IČO, else its name.
pub fn party_key(p: &Party) -> String {
    match &p.ico {
        Some(ico) => format!("ico:{ico}"),
        None => format!("name:{}", p.name),
    }
}

/// The existing contact of `p` under `rule`.
pub async fn contact<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    p: &Party,
    rule: ContactRule,
) -> Result<Option<contact::Model>, AppError> {
    use contact::Column;
    let q = || {
        contact::Entity::find()
            .filter(Column::SpaceId.eq(space))
            .order_by_asc(Column::CreatedAt)
    };
    if let Some(ico) = &p.ico {
        let found = q().filter(Column::Ico.eq(ico.as_str())).one(db).await?;
        if found.is_some() || rule == ContactRule::IcoOrName {
            return Ok(found);
        }
    }
    if rule == ContactRule::IcoOrName {
        return Ok(q()
            .filter(Column::Ico.is_null())
            .filter(Column::Name.eq(p.name.as_str()))
            .one(db)
            .await?);
    }
    // No contact holds the party's IČO here, so any contact with an IČO
    // would contradict it.
    let compatible = |q: sea_orm::Select<contact::Entity>| {
        let q = if p.ico.is_some() {
            q.filter(Column::Ico.is_null())
        } else {
            q
        };
        match &p.dic {
            Some(dic) => q.filter(Column::Dic.is_null().or(Column::Dic.eq(dic.as_str()))),
            None => q,
        }
    };
    if let Some(dic) = &p.dic {
        let found = compatible(q().filter(Column::Dic.eq(dic.as_str())))
            .one(db)
            .await?;
        if found.is_some() {
            return Ok(found);
        }
    }
    let name = p.name.trim().to_string();
    if name.is_empty() {
        return Ok(None);
    }
    Ok(compatible(q().filter(Expr::cust_with_values(
        "lower(btrim(name)) = lower($1)",
        [name],
    )))
    .one(db)
    .await?)
}

/// Received documents of the same supplier (snapshot): by IČO, else by name
/// among those without one.
fn same_supplier(p: &Party) -> SimpleExpr {
    match &p.ico {
        Some(ico) => Expr::cust_with_values("supplier_snapshot->>'ico' = $1", [ico.clone()]),
        None => Expr::cust_with_values(
            "supplier_snapshot->>'ico' IS NULL AND supplier_snapshot->>'name' = $1",
            [p.name.clone()],
        ),
    }
}

/// Issued: same type and number (drafts included). Received: same supplier
/// and supplier number.
pub async fn duplicate<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    plan: &Plan,
) -> Result<bool, AppError> {
    let q = Entity::find()
        .filter(Column::SpaceId.eq(space))
        .filter(Column::Direction.eq(plan.direction));
    let q = if plan.direction == ISSUED {
        q.filter(Column::DocType.eq(plan.doc_type.as_str()))
            .filter(Column::Number.eq(plan.number.as_str()))
    } else {
        q.filter(Column::SupplierNumber.eq(plan.number.as_str()))
            .filter(same_supplier(&plan.supplier))
    };
    Ok(q.select_only()
        .column(Column::Id)
        .into_tuple::<Uuid>()
        .one(db)
        .await?
        .is_some())
}

/// Types `doc_type` may link to (the 1f-a table).
pub fn targets(doc_type: DocType) -> Vec<&'static str> {
    related_types(doc_type).iter().map(|t| t.as_str()).collect()
}

/// The non-cancelled, non-draft original `OriginalDocumentReference` names.
pub async fn related<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    plan: &Plan,
) -> Result<Option<Uuid>, AppError> {
    let Some(reference) = &plan.original_ref else {
        return Ok(None);
    };
    let types = targets(plan.doc_type);
    if types.is_empty() {
        return Ok(None);
    }
    let q = Entity::find()
        .filter(Column::SpaceId.eq(space))
        .filter(Column::Direction.eq(plan.direction))
        .filter(Column::DocType.is_in(types))
        .filter(Column::Status.eq(Status::Issued.as_str()));
    let q = if plan.direction == ISSUED {
        q.filter(Column::Number.eq(reference.as_str()))
    } else {
        q.filter(Column::SupplierNumber.eq(reference.as_str()))
            .filter(same_supplier(&plan.supplier))
    };
    Ok(q.select_only()
        .column(Column::Id)
        .into_tuple::<Uuid>()
        .one(db)
        .await?)
}

/// Batch identity of a document: two entries with the same one are the
/// same document.
pub fn identity(plan: &Plan) -> String {
    if plan.direction == ISSUED {
        format!("issued|{}|{}", plan.doc_type.as_str(), plan.number)
    } else {
        format!("received|{}|{}", party_key(&plan.supplier), plan.number)
    }
}

/// `other` is the original `plan` references.
pub fn is_original_of(plan: &Plan, other: &Plan) -> bool {
    plan.original_ref.as_deref() == Some(other.number.as_str())
        && plan.direction == other.direction
        && targets(plan.doc_type).contains(&other.doc_type.as_str())
        && (plan.direction == ISSUED || party_key(&plan.supplier) == party_key(&other.supplier))
}

/// Import order: originals before the documents linking to them (every
/// link-table target has a lower rank).
pub fn rank(t: DocType) -> u8 {
    match t {
        DocType::Proforma => 0,
        DocType::CreditNote | DocType::DebitNote | DocType::AdvanceCreditNote => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_targets_rank_lower() {
        for t in DocType::ALL.into_iter().filter(|t| t.is_document_type()) {
            for target in related_types(t) {
                assert!(rank(*target) < rank(t), "{t:?} → {target:?}");
            }
        }
    }

    #[test]
    fn party_keys() {
        let mut p = Party {
            name: "A".into(),
            ico: Some("12345679".into()),
            ..Default::default()
        };
        assert_eq!(party_key(&p), "ico:12345679");
        p.ico = None;
        assert_eq!(party_key(&p), "name:A");
    }
}
