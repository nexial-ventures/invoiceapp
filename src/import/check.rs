//! The database side of a preview / confirm, shared by every source: batch
//! and stored duplicates, the contact match and the related original.

use std::collections::HashSet;

use sea_orm::DatabaseConnection;

use super::model::{Code, Plan};
use super::{batch, lookup};
use crate::error::AppError;
use crate::space::SpaceId;

pub const RELATED_NOT_FOUND: Code = "related_not_found";
pub const CONTACT_CREATED: Code = "contact_created";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Duplicate,
}

#[derive(Debug, Clone)]
pub struct Checked {
    pub status: Status,
    /// `Some(true)`: an existing contact matches the counterparty.
    pub contact_exists: Option<bool>,
    pub related_found: bool,
    /// [`CONTACT_CREATED`], [`RELATED_NOT_FOUND`].
    pub warnings: Vec<Code>,
}

/// One result per entry (`None` for an entry without a plan). `only`: run
/// the database lookups for these keys only (confirm: the selected ones);
/// the in-batch duplicate pass always covers every entry, a later entry
/// with an earlier one's identity being its duplicate.
pub async fn check(
    db: &DatabaseConnection,
    space: SpaceId,
    entries: &[(&str, Option<&Plan>)],
    only: Option<&HashSet<String>>,
) -> Result<Vec<Option<Checked>>, AppError> {
    let plans: Vec<&Plan> = entries.iter().filter_map(|(_, p)| *p).collect();
    // The database lookups run once for the whole batch; `looked[i]` is the
    // position of entry `i` in their input.
    let lookup_plans: Vec<&Plan> = entries
        .iter()
        .filter(|(key, _)| only.is_none_or(|o| o.contains(*key)))
        .filter_map(|(_, p)| *p)
        .collect();
    let stored = batch::duplicates(db, space, &lookup_plans).await?;
    let contacts = batch::contacts(db, space, &lookup_plans).await?;
    let originals = batch::related(db, space, &lookup_plans).await?;
    let mut looked = 0usize;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(entries.len());
    for (key, plan) in entries {
        let Some(plan) = plan else {
            out.push(None);
            continue;
        };
        let in_batch = !seen.insert(lookup::identity(plan));
        if only.is_some_and(|o| !o.contains(*key)) {
            out.push(Some(Checked {
                status: if in_batch {
                    Status::Duplicate
                } else {
                    Status::Ok
                },
                contact_exists: None,
                related_found: false,
                warnings: Vec::new(),
            }));
            continue;
        }
        let n = looked;
        looked += 1;
        let mut warnings = Vec::new();
        let duplicate = in_batch || stored.contains(&n);
        let contact_exists = plan.counterparty().map(|_| contacts.contains(&n));
        if contact_exists == Some(false) {
            warnings.push(CONTACT_CREATED);
        }
        let related_found = plan.original_ref.is_some()
            && (originals.contains(&n)
                || plans
                    .iter()
                    .any(|o| !std::ptr::eq(*o, *plan) && lookup::is_original_of(plan, o)));
        if plan.original_ref.is_some() && !related_found {
            warnings.push(RELATED_NOT_FOUND);
        }
        out.push(Some(Checked {
            status: if duplicate {
                Status::Duplicate
            } else {
                Status::Ok
            },
            contact_exists,
            related_found,
            warnings,
        }));
    }
    Ok(out)
}
