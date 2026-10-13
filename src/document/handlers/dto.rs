//! Request bodies only the REST action handlers take.

use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CancelInput {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct MarkSentInput {
    /// Default now.
    pub sent_at: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct InternalNoteInput {
    pub internal_note: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CreditNoteInput {
    /// At most 500 characters; required at issue.
    pub correction_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct UnlockInput {
    /// Required, 1–2000 characters (trimmed).
    pub reason: Option<String>,
}
