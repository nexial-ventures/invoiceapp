//! The exports' database side, shared by the CSV and the accounting
//! program files: the documents of a chunk with what their rows need
//! ([`Loaded`]), read inside the caller's snapshot transaction.

use std::collections::HashMap;

use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::source::Source;
use crate::document::entity::{document, payment, vat_recap};
use crate::error::AppError;

/// Documents loaded (and written) per query round.
pub const CHUNK: usize = 500;

fn group<T>(rows: Vec<T>, key: impl Fn(&T) -> Uuid) -> HashMap<Uuid, Vec<T>> {
    let mut map: HashMap<Uuid, Vec<T>> = HashMap::new();
    for r in rows {
        map.entry(key(&r)).or_default().push(r);
    }
    map
}

async fn by_id<C: ConnectionTrait>(
    db: &C,
    mut ids: Vec<Uuid>,
) -> Result<HashMap<Uuid, document::Model>, AppError> {
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(document::Entity::find()
        .filter(document::Column::Id.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .map(|d| (d.id, d))
        .collect())
}

/// One chunk of documents with their recap, originals and (CSV only)
/// payments.
pub struct Loaded {
    ids: Vec<Uuid>,
    docs: HashMap<Uuid, document::Model>,
    recap: HashMap<Uuid, Vec<vat_recap::Model>>,
    payments: HashMap<Uuid, Vec<payment::Model>>,
    parents: HashMap<Uuid, document::Model>,
}

impl Loaded {
    pub async fn load<C: ConnectionTrait>(
        db: &C,
        ids: &[Uuid],
        with_payments: bool,
    ) -> Result<Self, AppError> {
        let docs = by_id(db, ids.to_vec()).await?;
        let recap = group(
            vat_recap::Entity::find()
                .filter(vat_recap::Column::DocumentId.is_in(ids.to_vec()))
                .all(db)
                .await?,
            |r| r.document_id,
        );
        let payments = match with_payments {
            true => group(
                payment::Entity::find()
                    .filter(payment::Column::DocumentId.is_in(ids.to_vec()))
                    .all(db)
                    .await?,
                |p| p.document_id,
            ),
            false => HashMap::new(),
        };
        let parents = by_id(
            db,
            docs.values()
                .filter_map(|d| d.related_document_id)
                .collect(),
        )
        .await?;
        Ok(Self {
            ids: ids.to_vec(),
            docs,
            recap,
            payments,
            parents,
        })
    }

    /// The documents in `ids` order.
    pub fn sources<'a>(
        &'a self,
        categories: &'a HashMap<Uuid, String>,
    ) -> impl Iterator<Item = Source<'a>> {
        self.ids
            .iter()
            .filter_map(|id| self.docs.get(id))
            .map(|doc| Source {
                doc,
                recap: self.recap.get(&doc.id).map_or(&[], Vec::as_slice),
                payments: self.payments.get(&doc.id).map_or(&[], Vec::as_slice),
                parent: doc.related_document_id.and_then(|p| self.parents.get(&p)),
                category: doc
                    .category_id
                    .and_then(|c| categories.get(&c))
                    .map(String::as_str),
            })
    }
}

/// Ends a read-only snapshot that failed, logging a failed rollback.
pub async fn rollback(txn: DatabaseTransaction, label: &str) {
    if let Err(r) = txn.rollback().await {
        tracing::warn!(export = label, error = %r, "export rollback failed");
    }
}
