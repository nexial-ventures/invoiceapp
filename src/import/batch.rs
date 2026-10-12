//! The [`super::lookup`] existence checks of a whole preview / confirm, one
//! query per kind: the entries go in as one `jsonb` recordset and the query
//! returns the positions that match. The predicates are those of
//! `lookup::duplicate`, `lookup::contact` and `lookup::related`, so the
//! answers are the same; the per-entry transaction still runs those under
//! its lock.

use std::collections::HashSet;

use sea_orm::{ConnectionTrait, Statement, Value};
use serde_json::{Value as Json, json};

use super::model::{ContactRule, Plan};
use crate::document::line::Direction;
use crate::document::line::Status;
use crate::error::AppError;
use crate::space::SpaceId;

/// `d`'s supplier snapshot is `i`'s supplier: by IČO, else by name among
/// those without one (`lookup::same_supplier`).
const SAME_SUPPLIER: &str = "(i.ico IS NOT NULL AND d.supplier_snapshot->>'ico' = i.ico \
     OR i.ico IS NULL AND d.supplier_snapshot->>'ico' IS NULL \
     AND d.supplier_snapshot->>'name' = i.name)";

/// The positions (`n`) of the `rows` the query matches.
async fn matching<C: ConnectionTrait>(
    db: &C,
    sql: String,
    space: SpaceId,
    rows: Vec<Json>,
) -> Result<HashSet<usize>, AppError> {
    if rows.is_empty() {
        return Ok(HashSet::new());
    }
    let found = db
        .query_all(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            [space.into(), Value::Json(Some(Box::new(Json::Array(rows))))],
        ))
        .await?;
    found
        .iter()
        .map(|r| {
            let n: i64 = r.try_get("", "n")?;
            Ok(usize::try_from(n).map_err(anyhow::Error::from)?)
        })
        .collect()
}

/// `plans[n]` is already stored (`lookup::duplicate`).
pub async fn duplicates<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    plans: &[&Plan],
) -> Result<HashSet<usize>, AppError> {
    let rows = plans
        .iter()
        .enumerate()
        .map(|(n, p)| {
            json!({
                "n": n,
                "issued": p.direction == Direction::Issued,
                "direction": p.direction,
                "doc_type": p.doc_type.as_str(),
                "number": p.number,
                "ico": p.supplier.ico,
                "name": p.supplier.name,
            })
        })
        .collect();
    let sql = format!(
        "SELECT i.n FROM jsonb_to_recordset($2) AS i(n bigint, issued bool, direction text, \
         doc_type text, number text, ico text, name text) \
         WHERE EXISTS (SELECT 1 FROM documents d WHERE d.space_id = $1 \
         AND d.direction = i.direction \
         AND (i.issued AND d.doc_type = i.doc_type AND d.number = i.number \
         OR NOT i.issued AND d.supplier_number = i.number AND {SAME_SUPPLIER}))"
    );
    matching(db, sql, space, rows).await
}

/// An existing contact matches `plans[n]`'s counterparty
/// (`lookup::contact`); a plan without one is never in the set.
pub async fn contacts<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    plans: &[&Plan],
) -> Result<HashSet<usize>, AppError> {
    let rows = plans
        .iter()
        .enumerate()
        .filter_map(|(n, p)| {
            let party = p.counterparty()?;
            let by_dic = p.contact_rule == ContactRule::IcoDicName;
            // ISDOC matches the name exactly, CSV the trimmed one.
            let name = if by_dic {
                party.name.trim()
            } else {
                party.name.as_str()
            };
            Some(json!({
                "n": n,
                "by_dic": by_dic,
                "ico": party.ico,
                "dic": party.dic,
                "name": name,
            }))
        })
        .collect();
    // An IČO match wins under both rules; the DIČ / name fallbacks only
    // take a contact that does not contradict the party's IČO or DIČ.
    let sql = "SELECT i.n FROM jsonb_to_recordset($2) AS i(n bigint, by_dic bool, ico text, \
         dic text, name text) \
         WHERE EXISTS (SELECT 1 FROM contacts c WHERE c.space_id = $1 AND (c.ico = i.ico \
         OR NOT i.by_dic AND i.ico IS NULL AND c.ico IS NULL AND c.name = i.name \
         OR i.by_dic AND (i.ico IS NULL OR c.ico IS NULL) \
         AND (i.dic IS NULL OR c.dic IS NULL OR c.dic = i.dic) \
         AND (c.dic = i.dic OR i.name <> '' AND lower(btrim(c.name)) = lower(i.name))))"
        .to_string();
    matching(db, sql, space, rows).await
}

/// The original `plans[n]` references is stored (`lookup::related`).
pub async fn related<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    plans: &[&Plan],
) -> Result<HashSet<usize>, AppError> {
    // One row per (entry, type it may link to).
    let rows = plans
        .iter()
        .enumerate()
        .filter_map(|(n, p)| Some((n, p, p.original_ref.as_ref()?)))
        .flat_map(|(n, p, reference)| {
            super::lookup::targets(p.doc_type)
                .into_iter()
                .map(move |t| {
                    json!({
                        "n": n,
                        "issued": p.direction == Direction::Issued,
                        "direction": p.direction,
                        "doc_type": t,
                        "ref": reference,
                        "ico": p.supplier.ico,
                        "name": p.supplier.name,
                    })
                })
        })
        .collect();
    let sql = format!(
        "SELECT DISTINCT i.n FROM jsonb_to_recordset($2) AS i(n bigint, issued bool, \
         direction text, doc_type text, ref text, ico text, name text) \
         WHERE EXISTS (SELECT 1 FROM documents d WHERE d.space_id = $1 \
         AND d.direction = i.direction AND d.doc_type = i.doc_type AND d.status = '{}' \
         AND (i.issued AND d.number = i.ref \
         OR NOT i.issued AND d.supplier_number = i.ref AND {SAME_SUPPLIER}))",
        Status::Issued.as_str()
    );
    matching(db, sql, space, rows).await
}
