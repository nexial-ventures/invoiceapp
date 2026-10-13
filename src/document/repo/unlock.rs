//! `issued → draft` (`docs/api/documents.md`, "Unlock").

use chrono::{DateTime, FixedOffset};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set, TransactionTrait};
use uuid::Uuid;

use super::lifecycle::{ISSUED_ONLY, locked};
use crate::document::doc_type::DocType;
use crate::document::entity::document;
use crate::document::line::Status;
use crate::error::AppError;
use crate::space::SpaceId;

/// Who unlocks a document, and why.
pub struct Unlock {
    pub user: Uuid,
    pub reason: String,
}

/// Any issued document of ours but a native DDPP (its payment owns it).
/// Everything else stays: number, `sent_at`, payments, e-mails, documents
/// referencing it. A native document drops its snapshots (a draft shows
/// live data, the re-issue takes new ones) and a ČNB rate (re-resolved at
/// re-issue; a manual or copied rate is the user's and stays); an imported
/// one keeps both, as entered. Only the rendered archive (`pdf_*`) is
/// dropped — an uploaded original (`original_*`) is untouched.
/// Returns the archive key for the caller to remove once this committed.
pub async fn unlock(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    by: Unlock,
) -> Result<Option<String>, AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked(txn, space, id, ISSUED_ONLY, unlockable).await?;
                let archive = doc.pdf_path.clone();
                let native = !doc.imported;
                let cnb_rate = native && doc.exchange_rate_source.as_deref() == Some("cnb");
                let now: DateTime<FixedOffset> = chrono::Utc::now().into();
                // Re-issue tells an edited number from the kept one by it.
                let unlocked_number = doc.number.clone().filter(|_| native);
                let mut row: document::ActiveModel = doc.into();
                row.unlocked_number = Set(unlocked_number);
                row.status = Set(Status::Draft);
                row.unlocked_at = Set(Some(now));
                row.updated_at = Set(now);
                row.edited_since_unlock = Set(false);
                row.unlocked_by = Set(Some(by.user));
                row.unlock_reason = Set(Some(by.reason));
                if native {
                    row.supplier_snapshot = Set(None);
                    row.customer_snapshot = Set(None);
                    row.bank_snapshot = Set(None);
                }
                if cnb_rate {
                    row.exchange_rate = Set(None);
                    row.exchange_rate_date = Set(None);
                    row.exchange_rate_source = Set(None);
                }
                row.pdf_path = Set(None);
                row.pdf_sha256 = Set(None);
                row.pdf_size = Set(None);
                row.pdf_rendered_at = Set(None);
                row.update(txn).await?;
                Ok(archive)
            })
        })
        .await?)
}

fn unlockable(doc: &document::Model) -> bool {
    doc.doc_type != DocType::AdvanceTaxDoc || doc.imported
}

/// An imported document unlocked and not saved (`PUT`) since: its re-issue
/// keeps the lines, snapshots, rate and totals as they were entered.
pub fn untouched_import(doc: &document::Model) -> bool {
    doc.imported && doc.unlocked_at.is_some() && !doc.edited_since_unlock
}
