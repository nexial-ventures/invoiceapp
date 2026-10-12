//! Loading the documents `advance` lines reference, and the in-transaction
//! re-check that a deducted document is still available.

use std::collections::{HashMap, HashSet};

use anyhow::Context as _;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use super::ddpp_correction;
use crate::document::advance::{self, AdvanceSource};
use crate::document::correction::net_recap;
use crate::document::doc_type::DocType;
use crate::document::entity::{document, document_line, vat_recap};
use crate::document::line::Direction;
use crate::document::line::{AdvanceRow, LineData, Status};
use crate::error::{AppError, FieldErrors};
use crate::space::SpaceId;

/// `(referenced document, referencing invoice)` for every non-cancelled
/// invoice with an advance line on one of `ids`.
async fn referencing<C: ConnectionTrait>(
    db: &C,
    ids: &[Uuid],
) -> Result<Vec<(Uuid, Uuid)>, AppError> {
    let lines = document_line::Entity::find()
        .filter(document_line::Column::AdvanceDocumentId.is_in(ids.to_vec()))
        .all(db)
        .await?;
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let holders: Vec<Uuid> = lines.iter().map(|l| l.document_id).collect();
    let live: HashSet<Uuid> = document::Entity::find()
        .select_only()
        .column(document::Column::Id)
        .filter(document::Column::Id.is_in(holders))
        .filter(document::Column::DocType.eq(DocType::Invoice))
        .filter(document::Column::Status.ne(Status::Cancelled))
        .into_tuple::<Uuid>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    Ok(lines
        .into_iter()
        .filter(|l| live.contains(&l.document_id))
        .filter_map(|l| l.advance_document_id.map(|a| (a, l.document_id)))
        .collect())
}

/// The stored recap per document (rate desc); a DDPP's net of its issued
/// corrections — what it deducts.
async fn deducted_recaps<'a, C: ConnectionTrait>(
    db: &C,
    docs: impl Iterator<Item = &'a document::Model>,
) -> Result<HashMap<Uuid, Vec<AdvanceRow>>, AppError> {
    let (mut ids, mut ddpps) = (Vec::new(), Vec::new());
    for d in docs {
        ids.push(d.id);
        if d.doc_type == DocType::AdvanceTaxDoc {
            ddpps.push(d.id);
        }
    }
    let mut out: HashMap<Uuid, Vec<AdvanceRow>> = HashMap::new();
    for r in vat_recap::Entity::find()
        .filter(vat_recap::Column::DocumentId.is_in(ids))
        .order_by_desc(vat_recap::Column::VatRate)
        .all(db)
        .await?
    {
        out.entry(r.document_id).or_default().push(r.into());
    }
    for (ddpp, corrections) in ddpp_correction::credited(db, &ddpps, None).await? {
        if let Some(recap) = out.get_mut(&ddpp) {
            *recap = net_recap(recap, &corrections).context("DDPP net overflow")?;
        }
    }
    Ok(out)
}

/// The documents named by `ids`, with their recap and current references.
pub async fn load<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, AdvanceSource>, AppError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    // Received documents (and other spaces' documents) can never be
    // deducted: as if they did not exist.
    let docs = document::Entity::find()
        .filter(document::Column::SpaceId.eq(space))
        .filter(document::Column::Id.is_in(ids.to_vec()))
        .filter(document::Column::Direction.eq(Direction::Issued))
        .all(db)
        .await?;
    let mut recaps = deducted_recaps(db, docs.iter()).await?;
    let refs = referencing(db, ids).await?;
    let mut out = HashMap::with_capacity(docs.len());
    for d in docs {
        let supplier: Option<serde_json::Value> = d.supplier_snapshot.clone();
        let vat_payer = supplier
            .as_ref()
            .and_then(|s| s.get("vatPayer"))
            .and_then(serde_json::Value::as_bool);
        let recap = recaps.remove(&d.id).unwrap_or_default();
        let source = AdvanceSource {
            id: d.id,
            status: d.status,
            issues_ddpp: advance::issues_ddpp(vat_payer, d.vat_mode),
            recap,
            referenced_by: refs
                .iter()
                .filter(|(a, _)| *a == d.id)
                .map(|(_, holder)| *holder)
                .collect(),
            doc_type: d.doc_type,
            contact_id: d.contact_id,
            currency: d.currency,
            number: d.number,
            paid: d.paid,
        };
        out.insert(source.id, source);
    }
    Ok(out)
}

/// Inside the writing transaction: lock every referenced document (in id
/// order, so concurrent writers queue instead of deadlocking) and re-check
/// what may have changed since validation — still issued, not taken by
/// another invoice meanwhile, and (non-payer form) the paid amount unchanged.
pub async fn lock_and_recheck<C: ConnectionTrait>(
    txn: &C,
    space: SpaceId,
    document_id: Uuid,
    lines: &[LineData],
) -> Result<(), AppError> {
    let mut ids = advance::referenced_ids(lines);
    if ids.is_empty() {
        return Ok(());
    }
    ids.sort();
    ids.dedup();
    let locked: HashMap<Uuid, document::Model> = document::Entity::find()
        .filter(document::Column::SpaceId.eq(space))
        .filter(document::Column::Id.is_in(ids.clone()))
        .order_by_asc(document::Column::Id)
        .lock_exclusive()
        .all(txn)
        .await?
        .into_iter()
        .map(|d| (d.id, d))
        .collect();
    let refs = referencing(txn, &ids).await?;
    // DDPP corrections issue under the DDPP lock: re-read the net amounts.
    let fresh = deducted_recaps(txn, locked.values()).await?;
    let mut e = FieldErrors::new();
    for (i, line) in lines.iter().enumerate() {
        let LineData::Advance(a) = line else { continue };
        let field = format!("lines.{i}.advanceDocumentId");
        let doc = locked
            .get(&a.document_id)
            .context("advance document vanished")?;
        if doc.status != Status::Issued {
            e.add(&field, "invalid");
        } else if refs
            .iter()
            .any(|(adv, holder)| *adv == a.document_id && *holder != document_id)
        {
            e.add(&field, "duplicate");
        } else if doc.doc_type == DocType::Proforma
            && a.recap.first().map(|r| r.base) != Some(doc.paid)
        {
            return Err(AppError::Conflict("proforma payments changed".into()));
        } else if doc.doc_type == DocType::AdvanceTaxDoc
            && fresh.get(&a.document_id).unwrap_or(&Vec::new()) != &a.recap
        {
            return Err(AppError::Conflict("DDPP corrected meanwhile".into()));
        }
    }
    e.into_result()
}
