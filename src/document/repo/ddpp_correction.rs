//! DDPP corrections (`advance_credit_note`): the draft created from an issued
//! DDPP, the cap and exact VAT at issue, the guards that keep a corrected
//! DDPP's deduction consistent, and the net amounts settlement deducts.

use std::collections::HashMap;

use anyhow::Context as _;
use chrono::NaiveDate;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    QueryFilter, QueryOrder, TransactionTrait,
};
use uuid::Uuid;

use super::credit::draft_from;
use super::ddpp::ensure_not_deducted;
use super::{query, write};
use crate::document::compute::{self, Params, Totals};
use crate::document::correction::{ExactBasis, net_recap};
use crate::document::credit::exceeds_original;
use crate::document::doc_type::DocType;
use crate::document::entity::{document, vat_recap};
use crate::document::line::Direction;
use crate::document::line::{AdvanceRow, Status};
use crate::document::model::Existing;
use crate::error::AppError;
use crate::space::SpaceId;

/// An issued (not cancelled) DDPP of ours, imported ones included.
pub fn correctable(doc: &document::Model) -> Result<bool, AppError> {
    Ok(doc.doc_type == DocType::AdvanceTaxDoc
        && doc.direction == Direction::Issued
        && doc.status == Status::Issued)
}

/// Lock the DDPP, require it correctable and not deducted by a non-cancelled
/// invoice (`advance_settled` / `advance_in_use`).
async fn locked_ddpp<C: ConnectionTrait>(
    txn: &C,
    space: SpaceId,
    id: Uuid,
) -> Result<(), AppError> {
    let doc = query::lock(txn, space, id).await?;
    if !correctable(&doc)? {
        return Err(AppError::InvalidState);
    }
    ensure_not_deducted(txn, id).await
}

async fn recaps<C: ConnectionTrait>(
    db: &C,
    ids: Vec<Uuid>,
) -> Result<Vec<vat_recap::Model>, AppError> {
    Ok(vat_recap::Entity::find()
        .filter(vat_recap::Column::DocumentId.is_in(ids))
        .order_by_desc(vat_recap::Column::VatRate)
        .all(db)
        .await?)
}

/// The issued corrections' recap rows per DDPP in `ddpps`, except `except`.
pub async fn credited<C: ConnectionTrait>(
    db: &C,
    ddpps: &[Uuid],
    except: Option<Uuid>,
) -> Result<HashMap<Uuid, Vec<AdvanceRow>>, AppError> {
    let mut q = document::Entity::find()
        .filter(document::Column::RelatedDocumentId.is_in(ddpps.to_vec()))
        .filter(document::Column::DocType.eq(DocType::AdvanceCreditNote))
        .filter(document::Column::Direction.eq(Direction::Issued))
        .filter(document::Column::Status.eq(Status::Issued));
    if let Some(id) = except {
        q = q.filter(document::Column::Id.ne(id));
    }
    let notes = q.all(db).await?;
    let owner: HashMap<Uuid, Uuid> = notes
        .iter()
        .filter_map(|n| n.related_document_id.map(|d| (n.id, d)))
        .collect();
    let mut out: HashMap<Uuid, Vec<AdvanceRow>> = HashMap::new();
    for r in recaps(db, notes.iter().map(|n| n.id).collect()).await? {
        if let Some(ddpp) = owner.get(&r.document_id) {
            out.entry(*ddpp).or_default().push(r.into());
        }
    }
    Ok(out)
}

/// The DDPP's recap and what its issued corrections other than `except`
/// already credited.
pub async fn basis<C: ConnectionTrait>(
    db: &C,
    ddpp: Uuid,
    except: Option<Uuid>,
) -> Result<ExactBasis, AppError> {
    let recap = recaps(db, vec![ddpp])
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    let mut credited = credited(db, &[ddpp], except).await?;
    Ok(ExactBasis {
        ddpp: recap,
        credited: credited.remove(&ddpp).unwrap_or_default(),
    })
}

/// The exact-VAT basis of a saved draft (`PUT`, `/compute`): `Some` only for
/// a native DDPP correction.
pub async fn basis_for<C: ConnectionTrait>(
    db: &C,
    x: Option<&Existing>,
) -> Result<Option<ExactBasis>, AppError> {
    let native = x.filter(|x| x.doc_type == DocType::AdvanceCreditNote && !x.imported);
    match native.and_then(|x| x.related_document_id.map(|d| (x.id, d))) {
        Some((note, ddpp)) => Ok(Some(basis(db, ddpp, Some(note)).await?)),
        None => Ok(None),
    }
}

fn exact_error(o: crate::document::compute::Overflow) -> AppError {
    AppError::field(o.field(), "invalid")
}

/// Nothing of the DDPP is left once its issued corrections are netted.
async fn fully_corrected<C: ConnectionTrait>(db: &C, ddpp: Uuid) -> Result<bool, AppError> {
    let b = basis(db, ddpp, None).await?;
    Ok(net_recap(&b.ddpp, &b.credited)
        .context("DDPP net overflow")?
        .is_empty())
}

/// Why no correction can be made of `doc` now (`Document.correctionBlock`):
/// `advance_settled` > `advance_in_use` > `fully_corrected`; `None` when it
/// can be corrected or is not an issued DDPP of ours.
pub async fn block<C: ConnectionTrait>(
    db: &C,
    doc: &document::Model,
) -> Result<Option<&'static str>, AppError> {
    if !correctable(doc)? {
        return Ok(None);
    }
    match ensure_not_deducted(db, doc.id).await {
        Err(AppError::AdvanceSettled) => return Ok(Some("advance_settled")),
        Err(AppError::AdvanceInUse) => return Ok(Some("advance_in_use")),
        other => other?,
    }
    Ok(fully_corrected(db, doc.id)
        .await?
        .then_some("fully_corrected"))
}

/// A draft correction copying the DDPP's header and item lines.
pub async fn create(
    db: &DatabaseConnection,
    ddpp: &document::Model,
    correction_reason: Option<String>,
    today: NaiveDate,
) -> Result<Uuid, AppError> {
    let lines = query::load_lines(db, ddpp.id).await?;
    let data = draft_from(
        db,
        ddpp,
        DocType::AdvanceCreditNote,
        lines,
        correction_reason,
        today,
    )
    .await?;
    let mut totals = compute::evaluate(&data.lines, data.params())
        .map_err(AppError::Validation)?
        .totals;
    // Not-deducted is checked under the lock below.
    basis(db, ddpp.id, None)
        .await?
        .apply(&mut totals, data.params())
        .map_err(exact_error)?;
    let (space, ddpp_id) = (SpaceId::from_row(ddpp.space_id), ddpp.id);
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                locked_ddpp(txn, space, ddpp_id).await?;
                // Any correction of a fully corrected DDPP would exceed it.
                if fully_corrected(txn, ddpp_id).await? {
                    return Err(AppError::InvalidState);
                }
                write::create_in(txn, space, data, totals).await
            })
        })
        .await?)
}

/// At issue, with the DDPP locked: still correctable and not deducted, the
/// cap per rate (Σ issued corrections incl. this one ≤ the DDPP's base, no
/// other rate), then the exact VAT of a full correction.
pub async fn check_issue(
    txn: &DatabaseTransaction,
    space: SpaceId,
    note: Uuid,
    ddpp: Uuid,
    totals: &mut Totals,
    p: Params,
) -> Result<(), AppError> {
    locked_ddpp(txn, space, ddpp).await?;
    // Read under the DDPP lock: corrections of this DDPP issue serially.
    let basis = basis(txn, ddpp, Some(note)).await?;
    let cap: Vec<_> = basis.ddpp.iter().map(|r| (r.vat_rate, r.base)).collect();
    let all = basis.credited.iter().map(|r| (r.vat_rate, r.base)).chain(
        totals
            .recap
            .iter()
            .map(|r| (r.vat_rate.normalize(), r.base)),
    );
    if exceeds_original(&cap, all) {
        return Err(AppError::field("lines", "exceeds_original"));
    }
    basis.apply(totals, p).map_err(exact_error)
}

/// Issuing or cancelling a correction linked to a DDPP — imported ones too,
/// since they count in its net — is allowed only while the DDPP is not
/// deducted by a non-cancelled invoice (`advance_settled` / `advance_in_use`).
pub async fn check_linked(
    txn: &DatabaseTransaction,
    note: &document::Model,
) -> Result<(), AppError> {
    match note.related_document_id {
        Some(ddpp) => {
            query::lock(txn, SpaceId::from_row(note.space_id), ddpp).await?;
            ensure_not_deducted(txn, ddpp).await
        }
        None => Ok(()),
    }
}

/// A non-cancelled (draft or issued) correction of `ddpp` exists: its
/// payment may not be deleted, nor an imported DDPP cancelled
/// (`advance_in_use`).
pub async fn ensure_uncorrected<C: ConnectionTrait>(db: &C, ddpp: Uuid) -> Result<(), AppError> {
    let live = document::Entity::find()
        .filter(document::Column::RelatedDocumentId.eq(ddpp))
        .filter(document::Column::DocType.eq(DocType::AdvanceCreditNote))
        .filter(document::Column::Direction.eq(Direction::Issued))
        .filter(document::Column::Status.ne(Status::Cancelled))
        .one(db)
        .await?;
    match live {
        Some(_) => Err(AppError::AdvanceInUse),
        None => Ok(()),
    }
}
