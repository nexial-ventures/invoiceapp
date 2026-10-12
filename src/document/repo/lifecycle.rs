//! Changes allowed after issue: cancel, mark-sent, internal note, unlock.

use chrono::{DateTime, FixedOffset};
use sea_orm::{ActiveModelTrait, DatabaseConnection, DatabaseTransaction, Set, TransactionTrait};
use uuid::Uuid;

use super::{credit, ddpp_correction, query};
use crate::document::doc_type::DocType;
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::document::line::Status;
use crate::error::AppError;
use crate::space::SpaceId;

const ISSUED_ONLY: &[Status] = &[Status::Issued];

/// Lock the row, require one of `statuses` and `allowed`, apply `f`.
/// Anything else is `invalid_state`.
async fn on_status(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    statuses: &'static [Status],
    allowed: fn(&document::Model) -> bool,
    f: impl FnOnce(&mut document::ActiveModel) + Send + 'static,
) -> Result<(), AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked(txn, space, id, statuses, allowed).await?;
                apply(txn, doc, f).await
            })
        })
        .await?)
}

/// Lock the row; received documents are only recorded (no cancel / mark-sent
/// / e-mail).
async fn locked(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
    statuses: &[Status],
    allowed: fn(&document::Model) -> bool,
) -> Result<document::Model, AppError> {
    let doc = query::lock(txn, space, id).await?;
    if !statuses.contains(&doc.status) || doc.direction != Direction::Issued || !allowed(&doc) {
        return Err(AppError::InvalidState);
    }
    Ok(doc)
}

async fn apply(
    txn: &DatabaseTransaction,
    doc: document::Model,
    f: impl FnOnce(&mut document::ActiveModel),
) -> Result<(), AppError> {
    let mut row: document::ActiveModel = doc.into();
    f(&mut row);
    row.updated_at = Set(chrono::Utc::now().into());
    row.update(txn).await?;
    Ok(())
}

/// A native DDPP is only ever cancelled by deleting its payment (an imported
/// one has no payment link); a proforma only while nothing is paid on it.
fn cancellable(doc: &document::Model) -> bool {
    match doc.doc_type {
        DocType::AdvanceTaxDoc => doc.imported,
        DocType::Proforma => doc.paid.is_zero(),
        _ => true,
    }
}

/// Payments stay; the number stays used. Corrections keep their originals
/// consistent: a debit note only while the credit notes stay within the
/// lowered cap, a DDPP correction only while its DDPP is not deducted, an
/// (imported) DDPP only without live corrections.
pub async fn cancel(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    reason: Option<String>,
) -> Result<(), AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked(txn, space, id, ISSUED_ONLY, cancellable).await?;
                match doc.doc_type {
                    DocType::DebitNote => credit::check_debit_cancel(txn, &doc).await?,
                    DocType::AdvanceCreditNote => ddpp_correction::check_linked(txn, &doc).await?,
                    DocType::AdvanceTaxDoc => {
                        ddpp_correction::ensure_uncorrected(txn, doc.id).await?
                    }
                    _ => {}
                }
                apply(txn, doc, move |row| {
                    row.status = Set(Status::Cancelled);
                    row.cancelled_at = Set(Some(chrono::Utc::now().into()));
                    row.cancel_reason = Set(reason);
                })
                .await
            })
        })
        .await?)
}

/// Who unlocks a document, and why.
pub struct Unlock {
    pub user: Uuid,
    pub reason: String,
}

/// `issued → draft`, any issued document of ours (`docs/api/documents.md`,
/// "Unlock"). Everything else stays: number, `sent_at`, payments, e-mails,
/// documents referencing it. The snapshots go (a draft shows live data, the
/// re-issue takes new ones) and so does a ČNB rate (re-resolved at
/// re-issue; a manual or copied rate is the user's and stays). Only the
/// rendered archive (`pdf_*`) is dropped — an uploaded original
/// (`original_*`, imported documents) is untouched. Returns the archive key
/// for the caller to remove once this committed.
pub async fn unlock(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    by: Unlock,
) -> Result<Option<String>, AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked(txn, space, id, ISSUED_ONLY, |_| true).await?;
                let archive = doc.pdf_path.clone();
                let cnb_rate = doc.exchange_rate_source.as_deref() == Some("cnb");
                apply(txn, doc, move |row| {
                    row.status = Set(Status::Draft);
                    row.unlocked_at = Set(Some(chrono::Utc::now().into()));
                    row.unlocked_by = Set(Some(by.user));
                    row.unlock_reason = Set(Some(by.reason));
                    row.supplier_snapshot = Set(None);
                    row.customer_snapshot = Set(None);
                    row.bank_snapshot = Set(None);
                    if cnb_rate {
                        row.exchange_rate = Set(None);
                        row.exchange_rate_date = Set(None);
                        row.exchange_rate_source = Set(None);
                    }
                    row.pdf_path = Set(None);
                    row.pdf_sha256 = Set(None);
                    row.pdf_size = Set(None);
                    row.pdf_rendered_at = Set(None);
                })
                .await?;
                Ok(archive)
            })
        })
        .await?)
}

/// Idempotent: a repeated call overwrites `sent_at`.
pub async fn mark_sent(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    sent_at: DateTime<FixedOffset>,
) -> Result<(), AppError> {
    on_status(
        db,
        space,
        id,
        ISSUED_ONLY,
        |_| true,
        move |row| row.sent_at = Set(Some(sent_at)),
    )
    .await
}

/// After an e-mail went out: like mark-sent, but a cancelled document
/// (which can still be e-mailed) counts too.
pub async fn email_sent(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    sent_at: DateTime<FixedOffset>,
) -> Result<(), AppError> {
    on_status(
        db,
        space,
        id,
        &[Status::Issued, Status::Cancelled],
        |_| true,
        move |row| row.sent_at = Set(Some(sent_at)),
    )
    .await
}

/// Allowed in every status.
pub async fn set_internal_note(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    note: Option<String>,
) -> Result<(), AppError> {
    let doc = query::find(db, space, id).await?;
    let mut row: document::ActiveModel = doc.into();
    row.internal_note = Set(note);
    row.updated_at = Set(chrono::Utc::now().into());
    row.update(db).await?;
    Ok(())
}
