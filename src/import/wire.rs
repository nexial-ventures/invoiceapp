//! Preview and confirm response bodies shared by the import sources, and the
//! confirm loop's failure codes.

use std::collections::HashMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::check::{Checked, Status};
use super::model::{Code, Plan};
use crate::error::AppError;

#[derive(Debug, Serialize, ToSchema)]
pub struct Counterparty {
    pub name: String,
    pub ico: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreviewEntry {
    pub key: String,
    /// `ok` | `duplicate` | `error`.
    pub status: &'static str,
    pub error: Option<&'static str>,
    pub warnings: Vec<&'static str>,
    pub direction: Option<&'static str>,
    pub doc_type: Option<&'static str>,
    pub number: Option<String>,
    pub counterparty: Option<Counterparty>,
    /// `existing` | `new`.
    pub contact_match: Option<&'static str>,
    pub issue_date: Option<NaiveDate>,
    pub tax_point_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub currency: Option<String>,
    pub total: Option<Decimal>,
    pub has_pdf: bool,
    pub related_number: Option<String>,
    pub related_found: bool,
}

impl PreviewEntry {
    /// An entry that failed with `code`.
    pub fn error(key: String, code: Code) -> Self {
        Self {
            key,
            status: "error",
            error: Some(code),
            warnings: Vec::new(),
            direction: None,
            doc_type: None,
            number: None,
            counterparty: None,
            contact_match: None,
            issue_date: None,
            tax_point_date: None,
            due_date: None,
            currency: None,
            total: None,
            has_pdf: false,
            related_number: None,
            related_found: false,
        }
    }

    /// A planned entry; `warnings` are the source's, then the check's.
    pub fn planned(key: String, plan: &Plan, c: &Checked, mut warnings: Vec<Code>) -> Self {
        warnings.extend(&c.warnings);
        Self {
            status: match c.status {
                Status::Ok => "ok",
                Status::Duplicate => "duplicate",
            },
            error: None,
            warnings,
            direction: Some(plan.direction),
            doc_type: Some(plan.doc_type.as_str()),
            number: Some(plan.number.clone()),
            counterparty: plan.counterparty().map(|p| Counterparty {
                name: p.name.clone(),
                ico: p.ico.clone(),
            }),
            contact_match: c.contact_exists.map(|x| if x { "existing" } else { "new" }),
            issue_date: Some(plan.issue_date),
            tax_point_date: plan.tax_point_date,
            due_date: plan.due_date,
            currency: Some(plan.currency.clone()),
            total: Some(plan.gross),
            related_number: plan.original_ref.clone(),
            related_found: c.related_found,
            ..Self::error(key, "")
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmResult {
    pub key: String,
    /// `imported` | `skipped` | `failed`.
    pub status: &'static str,
    pub document_id: Option<Uuid>,
    pub error: Option<&'static str>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Confirmed {
    pub results: Vec<ConfirmResult>,
}

/// One result per key, in the given order; a key that was not imported nor
/// tried is `skipped`.
pub fn confirmed<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    done: &HashMap<&str, Result<Uuid, Code>>,
) -> Confirmed {
    let results = keys
        .into_iter()
        .map(|key| {
            let (status, document_id, error) = match done.get(key) {
                Some(Ok(id)) => ("imported", Some(*id), None),
                Some(Err(code)) => ("failed", None, Some(*code)),
                None => ("skipped", None, None),
            };
            ConfirmResult {
                key: key.to_string(),
                status,
                document_id,
                error,
            }
        })
        .collect();
    Confirmed { results }
}

/// The failure code of an entry whose store failed; anything unexpected is
/// logged and reported as `internal`.
pub fn failure(e: AppError) -> Code {
    match e {
        AppError::Duplicate => "duplicate",
        AppError::NumberTaken => "number_taken",
        // The space's storage quota has no room for this entry's PDF.
        AppError::QuotaExceeded(_) => "quota_exceeded",
        // Like `rate_unavailable`: a temporary outage, worth a retry later.
        AppError::StorageUnavailable(m) => {
            tracing::warn!(error = %m, "import: storage unavailable");
            "storage_unavailable"
        }
        other => {
            tracing::error!(error = %other, "import entry failed");
            "internal"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_failures_map_to_entry_codes() {
        let quota = AppError::QuotaExceeded("99.0 / 100 MB".into());
        assert_eq!(failure(quota), "quota_exceeded");
        assert_eq!(failure(AppError::NumberTaken), "number_taken");
        assert_eq!(failure(AppError::Duplicate), "duplicate");
        // A plain conflict (e.g. a unique race) is not an entry code.
        assert_eq!(failure(AppError::Conflict("x".into())), "internal");
        assert_eq!(
            failure(AppError::StorageUnavailable("down".into())),
            "storage_unavailable"
        );
        assert_eq!(failure(AppError::NotFound), "internal");
    }
}
