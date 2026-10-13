//! Re-issue of an unlocked draft (`docs/api/documents.md`, "Unlock"): the
//! documents that rely on it — its corrections, the invoices deducting it —
//! must still hold. Runs inside the issue transaction under the draft's row
//! lock; every writer of those dependents locks this row first (corrections
//! their original, deductions their source), so they queue behind it.

use rust_decimal::Decimal;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::{advance_sources, credit, ddpp_correction, query};
use crate::document::advance::{self, AdvanceCtx};
use crate::document::compute::Totals;
use crate::document::credit::exceeds_original;
use crate::document::doc_type::DocType;
use crate::document::entity::document;
use crate::document::line::{Direction, LineData, Status};
use crate::error::AppError;
use crate::space::SpaceId;

/// Non-cancelled corrections of `doc`: a native one copied its contact,
/// currency and VAT mode (changed → 409 `related_in_use`); the issued
/// credit notes must stay within the new amounts per rate (→ 409
/// `exceeds_original`). `totals`: what the re-issue stores.
pub(super) async fn check_corrections(
    txn: &DatabaseTransaction,
    space: SpaceId,
    doc: &document::Model,
    totals: &Totals,
) -> Result<(), AppError> {
    let corrections = document::Entity::find()
        .filter(document::Column::SpaceId.eq(space))
        .filter(document::Column::RelatedDocumentId.eq(doc.id))
        .filter(document::Column::Direction.eq(Direction::Issued))
        .filter(document::Column::DocType.is_in([
            DocType::CreditNote,
            DocType::DebitNote,
            DocType::AdvanceCreditNote,
        ]))
        .filter(document::Column::Status.ne(Status::Cancelled))
        .all(txn)
        .await?;
    if corrections.is_empty() {
        return Ok(());
    }
    if corrections
        .iter()
        .any(|c| !c.imported && !same_binding(c, doc))
    {
        return Err(AppError::RelatedInUse);
    }
    match doc.doc_type {
        DocType::Invoice | DocType::Simplified => {
            credit::credits_within_cap(txn, doc.id, Uuid::nil()).await
        }
        DocType::AdvanceTaxDoc => {
            let credited = ddpp_correction::credited(txn, &[doc.id], None).await?;
            let rows = credited.get(&doc.id).into_iter().flatten();
            let cap: Vec<(Decimal, Decimal)> = totals
                .recap
                .iter()
                .map(|r| (r.vat_rate.normalize(), r.base))
                .collect();
            if exceeds_original(&cap, rows.map(|r| (r.vat_rate.normalize(), r.base))) {
                return Err(AppError::ExceedsOriginal);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Whether a native correction's copied header still matches its original.
fn same_binding(correction: &document::Model, original: &document::Model) -> bool {
    correction.currency == original.currency
        && correction.vat_mode == original.vat_mode
        && correction.contact_id == original.contact_id
}

/// After the re-issued row and recap are written: every non-cancelled
/// invoice deducting `id` must deduct exactly what its advance line froze
/// (amounts, VAT recap, currency, party). Else 409 `advance_settled` (an
/// issued invoice) / `advance_in_use` (a draft one).
pub(super) async fn check_deductions(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
) -> Result<(), AppError> {
    let holders: Vec<Uuid> = advance_sources::referencing(txn, &[id])
        .await?
        .into_iter()
        .map(|(_, holder)| holder)
        .filter(|h| *h != id)
        .collect();
    if holders.is_empty() {
        return Ok(());
    }
    let sources = advance_sources::load(txn, space, &[id]).await?;
    for holder_id in holders {
        let holder = query::find(txn, space, holder_id).await?;
        let lines = query::load_lines(txn, holder_id).await?;
        let Some(frozen) = lines.into_iter().find_map(|l| match l {
            LineData::Advance(a) if a.document_id == id => Some(a),
            _ => None,
        }) else {
            continue;
        };
        let mut probe = vec![LineData::Advance(frozen.clone())];
        let ctx = AdvanceCtx {
            doc_type: holder.doc_type,
            vat_mode: holder.vat_mode,
            document_id: Some(holder.id),
            related_document_id: holder.related_document_id,
            contact_id: holder.contact_id,
            currency: &holder.currency,
            locale: &holder.locale,
            sources: &sources,
        };
        let still = advance::resolve(&mut probe, &ctx).is_empty()
            && matches!(probe.first(), Some(LineData::Advance(a)) if a.recap == frozen.recap);
        if !still {
            return Err(if holder.status == Status::Issued {
                AppError::AdvanceSettled
            } else {
                AppError::AdvanceInUse
            });
        }
    }
    Ok(())
}

/// After the re-issued row and recap are written: a debit note raises its
/// original's credit cap, so with this note at its new amounts the original's
/// issued credit notes must still fit (else 409 `exceeds_original`). (A DDPP
/// correction is checked by its issue path: refused while its DDPP is
/// deducted.)
pub(super) async fn check_as_debit_note(
    txn: &DatabaseTransaction,
    space: SpaceId,
    doc: &document::Model,
) -> Result<(), AppError> {
    let (DocType::DebitNote, Some(original)) = (doc.doc_type, doc.related_document_id) else {
        return Ok(());
    };
    // Lock order: correction → its original (the note is locked already).
    query::lock(txn, space, original).await?;
    credit::credits_within_cap(txn, original, Uuid::nil()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::line::VatMode;
    use crate::export::test_doc::doc;

    #[test]
    fn a_native_correction_is_bound_to_its_original() {
        let original = doc();
        let correction = doc();
        assert!(same_binding(&correction, &original));
        for change in [
            |d: &mut document::Model| d.currency = "EUR".into(),
            |d: &mut document::Model| d.vat_mode = VatMode::ReverseCharge,
            |d: &mut document::Model| d.contact_id = Some(Uuid::from_u128(7)),
        ] {
            let mut edited = original.clone();
            change(&mut edited);
            assert!(!same_binding(&correction, &edited));
        }
    }
}
