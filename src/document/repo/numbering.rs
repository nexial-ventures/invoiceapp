//! The number of an issued-direction document: what a draft save stores for
//! a manual number, and what the issue does (`docs/api/numbering.md`).

use chrono::Datelike;
use sea_orm::DatabaseTransaction;

use super::issue::variable_symbol;
use crate::contact::entity::contact;
use crate::document::entity::document;
use crate::error::AppError;
use crate::settings::pattern::Matched;
use crate::settings::repo::numbering;
use crate::space::SpaceId;

/// Where a document's number sits in the numbering scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Numbered {
    pub(super) number: String,
    pub(super) year: i32,
    /// `None`: in no sequence (imported, or a manual number no pattern renders).
    pub(super) seq: Option<i32>,
    pub(super) code: Option<String>,
    /// Entered by hand (not allocated, not the kept number of an unlock).
    pub(super) manual: bool,
}

/// What an issue does about the document's number.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Keep(Numbered),
    Manual(String),
    Allocate,
}

/// An imported document keeps its own number and never moves a counter. A
/// native draft keeps the number it had when it was unlocked (year, sequence
/// and code included, whatever was edited since); any other number on a
/// native draft was entered by hand; none at all allocates.
fn plan(doc: &document::Model, issue_year: i32) -> Result<Plan, AppError> {
    match (doc.number.as_deref(), doc.imported) {
        (Some(n), true) => Ok(Plan::Keep(Numbered {
            number: n.to_string(),
            year: doc.number_year.unwrap_or(issue_year),
            seq: None,
            code: None,
            manual: false,
        })),
        (None, true) => Err(AppError::field("number", "required")),
        (Some(n), false) if doc.unlocked_number.as_deref() == Some(n) => Ok(Plan::Keep(Numbered {
            number: n.to_string(),
            year: doc.number_year.unwrap_or(issue_year),
            seq: doc.number_seq,
            code: doc.number_code.clone(),
            manual: false,
        })),
        (Some(n), false) => Ok(Plan::Manual(n.to_string())),
        (None, false) => Ok(Plan::Allocate),
    }
}

/// `number` read back through the series' patterns; no match → no sequence,
/// the year of the issue date.
fn placed(number: String, m: Option<Matched>, fallback_year: i32, manual: bool) -> Numbered {
    match m {
        Some(m) => Numbered {
            number,
            year: m.year,
            seq: Some(m.seq),
            code: m.code,
            manual,
        },
        None => Numbered {
            number,
            year: fallback_year,
            seq: None,
            code: None,
            manual,
        },
    }
}

/// The number the issue stores (allocating one when the draft has none).
pub(super) async fn at_issue(
    txn: &DatabaseTransaction,
    space: SpaceId,
    doc: &document::Model,
    customer: Option<&contact::Model>,
) -> Result<Numbered, AppError> {
    let issue_year = doc.issue_date.year();
    match plan(doc, issue_year)? {
        Plan::Keep(n) => Ok(n),
        Plan::Manual(n) => {
            let m = numbering::match_number(txn, space, doc.doc_type, customer, &n).await?;
            Ok(placed(n, m, issue_year, true))
        }
        Plan::Allocate => {
            let a = numbering::allocate(txn, space, doc.doc_type, issue_year, customer).await?;
            Ok(Numbered {
                number: a.number,
                year: a.year,
                seq: Some(a.seq),
                code: a.code,
                manual: false,
            })
        }
    }
}

/// At a re-issue: the replaced number (`unlocked` → `previous_number`) when
/// it changed, and the variable symbol — re-derived when it was derived
/// from the replaced number, else as entered (or derived when empty).
pub(super) fn reissue_symbols(
    unlocked: Option<&str>,
    number: &str,
    entered_vs: Option<&str>,
) -> (Option<String>, String) {
    let replaced = unlocked.filter(|old| *old != number);
    let vs = match entered_vs {
        Some(vs) if replaced.is_some_and(|old| vs == variable_symbol(old)) => {
            variable_symbol(number)
        }
        Some(vs) => vs.to_string(),
        None => variable_symbol(number),
    };
    (replaced.map(str::to_string), vs)
}

#[cfg(test)]
#[path = "numbering_tests.rs"]
mod tests;
