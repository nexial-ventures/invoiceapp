//! The CSV export file: its rate columns and the rows of every chunk, built
//! inside the caller's snapshot transaction ([`crate::export::load`]).

use std::collections::HashMap;

use rust_decimal::Decimal;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect,
};
use uuid::Uuid;

use super::export_row::out_row;
use super::write;
use crate::document::entity::vat_recap;
use crate::error::AppError;
use crate::export::load::{CHUNK, Loaded, rollback};
use crate::settings::repo::{categories, vat_rates};
use crate::space::SpaceId;

/// Settings → VAT rates plus every rate the documents' recap uses.
pub async fn rates<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    ids: &[Uuid],
) -> Result<Vec<Decimal>, AppError> {
    let mut rates: Vec<Decimal> = vat_rates::list(db, space)
        .await?
        .into_iter()
        .map(|r| r.rate)
        .collect();
    for chunk in ids.chunks(CHUNK * 10) {
        let used: Vec<Decimal> = vat_recap::Entity::find()
            .select_only()
            .column(vat_recap::Column::VatRate)
            .distinct()
            .filter(vat_recap::Column::DocumentId.is_in(chunk.to_vec()))
            .into_tuple()
            .all(db)
            .await?;
        rates.extend(used);
    }
    Ok(rates)
}

/// The CSV rows of `ids`, in that order. A document whose row cannot be
/// built (an undecodable snapshot, a foreign amount without a CZK value…)
/// is skipped and logged: the rest of the file stays complete.
async fn chunk<C: ConnectionTrait>(db: &C, ids: &[Uuid], w: &Walk) -> Result<Vec<u8>, AppError> {
    let loaded = Loaded::load(db, ids, true).await?;
    let mut rows = Vec::with_capacity(ids.len());
    for s in loaded.sources(&w.categories) {
        match out_row(&s) {
            Ok(row) => rows.push(row),
            Err(e) => {
                tracing::error!(export = w.label, document = %s.doc.id, error = %format!("{e:#}"), "export skipped a document");
            }
        }
    }
    Ok(write::body(&w.rates, &rows)?)
}

async fn prepare(
    txn: &DatabaseTransaction,
    space: SpaceId,
    ids: &[Uuid],
) -> Result<(Vec<Decimal>, Vec<u8>, HashMap<Uuid, String>), AppError> {
    let rates = rates(txn, space, ids).await?;
    let head = write::head(&rates)?;
    let categories = categories::list(txn, space)
        .await?
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    Ok((rates, head, categories))
}

/// What every chunk shares: the header's rates and the category names.
struct Walk {
    rates: Vec<Decimal>,
    categories: HashMap<Uuid, String>,
    /// Which export, for the logs.
    label: &'static str,
}

/// The CSV of `ids` (already ordered), built whole inside the snapshot
/// `txn`, which is then committed (rolled back on a failure): the response
/// goes out after the transaction ended, so a slow client never holds a
/// snapshot open on a pooled connection. `label` names the export in the
/// logs.
pub async fn csv(
    txn: DatabaseTransaction,
    space: SpaceId,
    ids: Vec<Uuid>,
    label: &'static str,
) -> Result<Vec<u8>, AppError> {
    match build(&txn, space, &ids, label).await {
        Ok(bytes) => {
            txn.commit().await?;
            Ok(bytes)
        }
        Err(e) => {
            rollback(txn, label).await;
            Err(e)
        }
    }
}

async fn build(
    txn: &DatabaseTransaction,
    space: SpaceId,
    ids: &[Uuid],
    label: &'static str,
) -> Result<Vec<u8>, AppError> {
    let (rates, mut out, categories) = prepare(txn, space, ids).await?;
    let w = Walk {
        rates,
        categories,
        label,
    };
    for ids in ids.chunks(CHUNK) {
        out.extend(chunk(txn, ids, &w).await?);
    }
    Ok(out)
}
