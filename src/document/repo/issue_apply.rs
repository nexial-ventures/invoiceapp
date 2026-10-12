//! The transactional part of `draft → issued` (see [`super::issue::issue`]):
//! the dry run that plans the PDF payload, and the final transaction.

use anyhow::Context as _;
use bytes::Bytes;
use chrono::Datelike;
use sea_orm::{ActiveModelTrait, DatabaseConnection, DatabaseTransaction, Set, TransactionTrait};
use uuid::Uuid;

use super::issue::{Prepared, customer, json, supplier, variable_symbol};
use super::{advance_sources, credit, ddpp_correction, lines, query, view, write};
use crate::document::advance;
use crate::document::entity::document;
use crate::document::line::Status;
use crate::document::snapshot::BankSnapshot;
use crate::error::{AppError, number_violation};
use crate::pdf::payload::{self, Payload};
use crate::pdf::{archive, source};
use crate::settings::doc_type::DocType;
use crate::settings::repo::number_series;
use crate::space::SpaceId;

/// The issued document's PDF payload as the dry run built it.
pub(super) struct Planned {
    pub(super) payload: Payload,
    /// The serialized payload: what the final transaction must reproduce.
    pub(super) json: String,
    pub(super) year: i32,
}

/// Dry run: apply the issue in a transaction that is always rolled back,
/// and build the payload of the result.
pub(super) async fn plan(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    p: &Prepared,
) -> Result<Planned, AppError> {
    let txn = db.begin().await?;
    let result = async {
        apply(&txn, space, id, p).await?;
        issued_payload(&txn, space, id).await
    }
    .await;
    if let Err(e) = txn.rollback().await {
        tracing::warn!(error = %e, "rollback issue dry run");
    }
    result
}

async fn issued_payload(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
) -> Result<Planned, AppError> {
    let src = source::load(txn, space, id).await?;
    let payload = payload::build(&src.input()?)?;
    let json = serde_json::to_string(&payload).context("serialize PDF payload")?;
    let year = src
        .row
        .number_year
        .with_context(|| format!("issued document {id} has no number year"))?;
    Ok(Planned {
        payload,
        json,
        year,
    })
}

/// The final transaction: apply again and record the archive when the
/// result still matches what was rendered; `false` when it does not.
pub(super) async fn commit_planned(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
    p: &Prepared,
    rendered: &str,
    rel: &str,
    bytes: &Bytes,
) -> Result<bool, AppError> {
    apply(txn, space, id, p).await?;
    if issued_payload(txn, space, id).await?.json != rendered {
        return Ok(false);
    }
    // The row was a draft under our lock, so it has no archive yet.
    if !archive::record(txn, id, rel, bytes).await? {
        return Err(AppError::Conflict("document archived while issuing".into()));
    }
    Ok(true)
}

/// Commit on `Ok(true)`, roll back otherwise.
pub(super) async fn finish(
    txn: DatabaseTransaction,
    result: Result<bool, AppError>,
) -> Result<bool, AppError> {
    match result {
        Ok(true) => {
            txn.commit().await?;
            Ok(true)
        }
        other => {
            if let Err(e) = txn.rollback().await {
                tracing::warn!(error = %e, "rollback issue");
            }
            other
        }
    }
}

/// Lock the draft, re-check it and mark it issued: number, snapshots,
/// totals, re-resolved deductions. Runs in a transaction the caller commits
/// (the final one) or rolls back (the dry run that builds the PDF payload).
pub(super) async fn apply(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
    p: &Prepared,
) -> Result<(), AppError> {
    let doc = query::lock(txn, space, id).await?;
    if view::status(&doc)? != Status::Draft {
        return Err(AppError::InvalidState);
    }
    // Edited between validation and the lock: what we validated is stale.
    if doc.updated_at != p.seen_updated_at {
        return Err(AppError::Conflict("document changed while issuing".into()));
    }
    let mut totals = p.evaluated.totals.clone();
    advance_sources::lock_and_recheck(txn, space, id, &p.lines).await?;
    // An imported correction documents amounts as they were: no cap.
    if p.doc_type.is_correction() && !doc.imported {
        let original = doc
            .related_document_id
            .context("correction without its original")?;
        match p.doc_type {
            DocType::CreditNote => credit::check_cap(txn, space, id, original, &totals).await?,
            // No cap, but the original must still be correctable.
            DocType::DebitNote => {
                credit::locked_original(txn, space, original).await?;
            }
            DocType::AdvanceCreditNote => {
                ddpp_correction::check_issue(txn, space, id, original, &mut totals, p.params)
                    .await?
            }
            _ => {}
        }
    } else if p.doc_type == DocType::AdvanceCreditNote {
        // An imported correction linked to a DDPP counts in its net too.
        ddpp_correction::check_linked(txn, &doc).await?;
    }
    // An imported document keeps its own number and never moves the counter.
    let (number, seq) = match (&doc.number, doc.imported) {
        (Some(n), true) => (n.clone(), None),
        (None, true) => return Err(AppError::field("number", "required")),
        (_, false) => {
            let (n, seq) =
                number_series::allocate_number(txn, space, p.doc_type, doc.issue_date.year())
                    .await?;
            (n, Some(seq))
        }
    };
    let mut row: document::ActiveModel = doc.clone().into();
    row.status = Set(Status::Issued.as_str().into());
    row.variable_symbol = Set(Some(
        doc.variable_symbol
            .clone()
            .unwrap_or_else(|| variable_symbol(&number)),
    ));
    row.number = Set(Some(number));
    row.number_year = Set(Some(doc.issue_date.year()));
    row.number_seq = Set(seq);
    row.exchange_rate = Set(p.rate.rate);
    row.exchange_rate_date = Set(p.rate.date);
    row.exchange_rate_source = Set(p.rate.source.map(str::to_string));
    row.supplier_snapshot = Set(Some(json(&supplier(&p.company))?));
    row.customer_snapshot = Set(p
        .customer
        .as_ref()
        .map(|c| json(&customer(c)))
        .transpose()?);
    row.bank_snapshot = Set(p
        .bank
        .as_ref()
        .map(|b| {
            json(&BankSnapshot {
                account_number: b.account_number.clone(),
                iban: b.iban.clone(),
                bic: b.bic.clone(),
            })
        })
        .transpose()?);
    write::apply_totals(&mut row, &totals);
    row.updated_at = Set(chrono::Utc::now().into());
    row.update(txn)
        .await
        .map_err(|e| number_violation(e, || AppError::NumberTaken))?;
    if !advance::referenced_ids(&p.lines).is_empty() {
        lines::replace(txn, id, &p.lines).await?;
    }
    lines::replace_recap(txn, id, &totals).await?;
    Ok(())
}
