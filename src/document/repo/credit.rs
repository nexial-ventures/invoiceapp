//! Credit and debit notes of an invoice / simplified document: the drafts
//! created from it, the credit cap checked at issue (raised by issued debit
//! notes) and the cap re-check when a debit note is cancelled.

use anyhow::Context as _;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    QueryFilter, TransactionTrait,
};
use uuid::Uuid;

use super::{context, ddpp_correction, query, view, write};
use crate::document::compute::{self, Params, Totals};
use crate::document::correction::sum_bases;
use crate::document::credit::{copy_lines, exceeds_original};
use crate::document::entity::{document, vat_recap};
use crate::document::line::{LineData, PaymentMethod, RoundingMode, Status, VatMode};
use crate::document::model::DocumentData;
use crate::error::AppError;
use crate::settings::doc_type::{DocType, ISSUED};
use crate::space::SpaceId;

/// An issued (not draft, not cancelled) invoice or simplified document of
/// ours, imported ones included: what credit and debit notes correct.
pub fn correctable(doc: &document::Model) -> Result<bool, AppError> {
    Ok(
        (doc.doc_type == DocType::Invoice.as_str() || doc.doc_type == DocType::Simplified.as_str())
            && doc.direction == ISSUED
            && view::status(doc)? == Status::Issued,
    )
}

/// Lock `id` and require a correctable document (else `invalid_state`).
pub async fn locked_original<C: ConnectionTrait>(
    txn: &C,
    space: SpaceId,
    id: Uuid,
) -> Result<document::Model, AppError> {
    let doc = query::lock(txn, space, id).await?;
    if !correctable(&doc)? {
        return Err(AppError::InvalidState);
    }
    Ok(doc)
}

/// The header a correction copies from `orig`: contact, currency, locale,
/// VAT mode, bank, payment method and the original's rate.
pub async fn draft_from(
    db: &DatabaseConnection,
    orig: &document::Model,
    doc_type: DocType,
    lines: Vec<LineData>,
    correction_reason: Option<String>,
    today: NaiveDate,
) -> Result<DocumentData, AppError> {
    let vat_mode = VatMode::parse(&orig.vat_mode)
        .with_context(|| format!("document {} has unknown vat mode", orig.id))?;
    Ok(DocumentData {
        doc_type,
        related_document_id: Some(orig.id),
        correction_reason,
        contact_id: orig.contact_id,
        issue_date: today,
        tax_point_date: Some(today),
        due_date: context::due_date(db, SpaceId::from_row(orig.space_id), orig.contact_id, today)
            .await?,
        exchange_rate: orig.exchange_rate.filter(|_| orig.currency != "CZK"),
        currency: orig.currency.clone(),
        locale: orig.locale.clone(),
        vat_mode,
        bank_account_id: orig.bank_account_id,
        payment_method: PaymentMethod::parse(&orig.payment_method)
            .unwrap_or(PaymentMethod::BankTransfer),
        variable_symbol: None,
        constant_symbol: None,
        order_ref: None,
        header_note: None,
        footer_note: None,
        internal_note: None,
        rounding_mode: view::rounding_mode(orig)?,
        lines,
        imported: false,
        number: None,
        meta: Default::default(),
    })
}

/// `POST …/credit-note`: a credit note of an invoice / simplified document
/// (all lines copied, advance lines dropped), or the correction of a DDPP.
pub async fn create(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    correction_reason: Option<String>,
    today: NaiveDate,
) -> Result<Uuid, AppError> {
    let orig = query::find(db, space, id).await?;
    if ddpp_correction::correctable(&orig)? {
        return ddpp_correction::create(db, &orig, correction_reason, today).await;
    }
    let lines = copy_lines(&query::load_lines(db, id).await?);
    create_note(
        db,
        orig,
        DocType::CreditNote,
        lines,
        correction_reason,
        today,
    )
    .await
}

/// `POST …/debit-note`: an empty debit-note draft (the user enters the
/// additional charge).
pub async fn create_debit(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    correction_reason: Option<String>,
    today: NaiveDate,
) -> Result<Uuid, AppError> {
    let orig = query::find(db, space, id).await?;
    create_note(
        db,
        orig,
        DocType::DebitNote,
        Vec::new(),
        correction_reason,
        today,
    )
    .await
}

async fn create_note(
    db: &DatabaseConnection,
    orig: document::Model,
    doc_type: DocType,
    lines: Vec<LineData>,
    correction_reason: Option<String>,
    today: NaiveDate,
) -> Result<Uuid, AppError> {
    if !correctable(&orig)? {
        return Err(AppError::InvalidState);
    }
    let data = draft_from(db, &orig, doc_type, lines, correction_reason, today).await?;
    let totals = compute::evaluate(&data.lines, data.params())
        .map_err(AppError::Validation)?
        .totals;
    let (space, orig_id) = (SpaceId::from_row(orig.space_id), orig.id);
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                locked_original(txn, space, orig_id).await?;
                write::create_in(txn, space, data, totals).await
            })
        })
        .await?)
}

/// Issued notes of `doc_type` correcting `original`, other than `except`.
async fn issued_notes<C: ConnectionTrait>(
    db: &C,
    original: Uuid,
    doc_type: DocType,
    except: Uuid,
) -> Result<Vec<(Decimal, Decimal)>, AppError> {
    let ids: Vec<Uuid> = document::Entity::find()
        .filter(document::Column::RelatedDocumentId.eq(original))
        .filter(document::Column::DocType.eq(doc_type.as_str()))
        .filter(document::Column::Status.eq(Status::Issued.as_str()))
        .filter(document::Column::Id.ne(except))
        .all(db)
        .await?
        .into_iter()
        .map(|d| d.id)
        .collect();
    Ok(vat_recap::Entity::find()
        .filter(vat_recap::Column::DocumentId.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .map(|r| (r.vat_rate.normalize(), r.base))
        .collect())
}

/// The credit cap per rate: the original's item bases plus the bases of its
/// issued debit notes (other than `except`).
async fn cap<C: ConnectionTrait>(
    db: &C,
    original: Uuid,
    except: Uuid,
) -> Result<Vec<(Decimal, Decimal)>, AppError> {
    let debits = issued_notes(db, original, DocType::DebitNote, except).await?;
    let mut rows = item_bases(db, original).await?;
    rows.extend(debits);
    Ok(sum_bases(rows).context("credit cap overflow")?)
}

/// Every issued credit note of the original plus the one being issued
/// (`own`) may not exceed the cap per rate. Locks the original, so
/// concurrent credit-note issues queue.
pub async fn check_cap(
    txn: &DatabaseTransaction,
    space: SpaceId,
    credit_note_id: Uuid,
    original_id: Uuid,
    own: &Totals,
) -> Result<(), AppError> {
    locked_original(txn, space, original_id).await?;
    let cap = cap(txn, original_id, credit_note_id).await?;
    let credited = issued_notes(txn, original_id, DocType::CreditNote, credit_note_id).await?;
    let credited = credited
        .into_iter()
        .chain(own.recap.iter().map(|r| (r.vat_rate.normalize(), r.base)));
    if exceeds_original(&cap, credited) {
        return Err(AppError::field("lines", "exceeds_original"));
    }
    Ok(())
}

/// Cancelling an issued debit note lowers the cap: refused (409
/// `exceeds_original`) when the issued credit notes would then exceed it.
pub async fn check_debit_cancel(
    txn: &DatabaseTransaction,
    debit_note: &document::Model,
) -> Result<(), AppError> {
    // Imported debit notes linked to the original raise its cap too.
    let Some(original_id) = debit_note.related_document_id else {
        return Ok(());
    };
    query::lock(txn, SpaceId::from_row(debit_note.space_id), original_id).await?;
    let cap = cap(txn, original_id, debit_note.id).await?;
    let credited = issued_notes(txn, original_id, DocType::CreditNote, Uuid::nil()).await?;
    if exceeds_original(&cap, credited) {
        return Err(AppError::ExceedsOriginal);
    }
    Ok(())
}

/// The original's item base per rate (before any advance deduction).
async fn item_bases<C: ConnectionTrait>(
    db: &C,
    original: Uuid,
) -> Result<Vec<(Decimal, Decimal)>, AppError> {
    let lines = query::load_lines(db, original).await?;
    let items = lines.iter().filter_map(|l| match l {
        LineData::Item(i) => {
            compute::item_base(i.quantity, i.unit_price, i.discount_pct).map(|b| (i.vat_rate, b))
        }
        _ => None,
    });
    // Recap rows with vat 0 — only the per-rate base sums matter here.
    let params = Params {
        vat_mode: VatMode::Exempt,
        is_czk: true,
        exchange_rate: None,
        rounding: RoundingMode::None,
    };
    let totals = compute::totals(items, params)
        .map_err(|_| anyhow::anyhow!("stored document {original} totals overflow"))?;
    Ok(totals
        .recap
        .into_iter()
        .map(|r| (r.vat_rate.normalize(), r.base))
        .collect())
}
