//! Tool results: the REST wire JSON as pretty text + `structuredContent`,
//! failures as `isError` results carrying the REST error body.

use anyhow::Context as _;
use rmcp::model::{CallToolResult, ContentBlock};
use serde::Serialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::document::line::Direction;
use crate::error::AppError;
use crate::paging::ListQuery as Paging;
use crate::pdf::archive::serves_original;

// Smaller than the REST page (`paging::DEFAULT_LIMIT` / `MAX_LIMIT`): every
// result row lands in the client model's context window as pretty JSON, so a
// page stays a few thousand tokens. The rest of the normalization (trimmed
// `q`, `offset` cap) is shared through `paging::ListQuery`.
pub const DEFAULT_LIMIT: u64 = 20;
pub const MAX_LIMIT: u64 = 100;

/// `(search, limit, offset)` as REST normalizes them, with the MCP page size
/// (default 20, at most 100).
pub fn paging(
    q: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> (Option<String>, u64, u64) {
    Paging {
        q,
        limit: Some(limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT)),
        offset,
    }
    .normalized()
}

/// Where the client downloads the PDF (same Bearer token), relative to the
/// server origin; `None` when `GET …/pdf` would answer `pdf_missing` (a
/// received / imported document without an uploaded original).
pub fn pdf_url(
    id: Uuid,
    direction: Direction,
    imported: bool,
    has_original: bool,
) -> Option<String> {
    (!serves_original(direction, imported) || has_original)
        .then(|| format!("/api/documents/{id}/pdf"))
}

/// The result of a committed write: the re-read `view`, or — when only that
/// read failed — `{ id, readError }` as a success, so a client does not retry
/// (and duplicate) a write that happened. The read failure is logged.
pub fn written(id: Uuid, view: Result<Value, AppError>) -> Value {
    view.unwrap_or_else(|e| {
        e.log();
        serde_json::json!({ "id": id, "readError": e.body() })
    })
}

/// `value` (a JSON object) with `key` set.
pub fn with(mut value: Value, key: &str, extra: impl Into<Value>) -> Value {
    if let Value::Object(map) = &mut value {
        map.insert(key.to_string(), extra.into());
    }
    value
}

/// The serialized wire value; a failure is internal.
pub fn to_value<T: Serialize>(value: &T) -> Result<Value, AppError> {
    Ok(serde_json::to_value(value).context("serialize tool result")?)
}

/// `structuredContent` must be an object: anything else is wrapped as `{ "items": … }`.
fn object(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        other => Value::Object(Map::from_iter([("items".to_string(), other)])),
    }
}

fn result(value: Value, is_error: bool) -> CallToolResult {
    let value = object(value);
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    let mut out = if is_error {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    };
    out.content = vec![ContentBlock::text(text)];
    out
}

/// Success → the value; an [`AppError`] → a tool error with the REST error
/// body (logged like a REST failure; internal causes never leave the server).
pub fn respond<T: Serialize>(outcome: Result<T, AppError>) -> CallToolResult {
    match outcome.and_then(|v| to_value(&v)) {
        Ok(value) => result(value, false),
        Err(e) => {
            e.log();
            match to_value(&e.body()) {
                Ok(body) => result(body, true),
                Err(_) => result(serde_json::json!({ "code": "internal" }), true),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::FieldErrors;
    use serde_json::json;

    fn text(r: &CallToolResult) -> Value {
        let raw = r.content[0].as_text().expect("text content").text.clone();
        serde_json::from_str(&raw).expect("json text")
    }

    #[test]
    fn paging_defaults_and_clamps() {
        assert_eq!(paging(None, None, None), (None, 20, 0));
        assert_eq!(
            paging(Some(" a ".into()), Some(0), Some(3)),
            (Some("a".into()), 1, 3)
        );
        assert_eq!(paging(None, Some(10_000), None), (None, 100, 0));
    }

    #[test]
    fn pdf_url_follows_the_download_rules() {
        let id = Uuid::nil();
        let url = Some(format!("/api/documents/{id}/pdf"));
        assert_eq!(pdf_url(id, Direction::Issued, false, false), url);
        assert_eq!(pdf_url(id, Direction::Received, false, true), url);
        assert_eq!(pdf_url(id, Direction::Issued, true, true), url);
        assert_eq!(pdf_url(id, Direction::Received, false, false), None);
        assert_eq!(pdf_url(id, Direction::Issued, true, false), None);
    }

    #[test]
    fn written_keeps_success_when_the_read_fails() {
        let id = Uuid::nil();
        assert_eq!(written(id, Ok(json!({ "a": 1 }))), json!({ "a": 1 }));
        let v = written(id, Err(AppError::Internal(anyhow::anyhow!("db gone"))));
        assert_eq!(v, json!({ "id": id, "readError": { "code": "internal" } }));
    }

    #[test]
    fn success_carries_text_and_structured_content() {
        let r = respond::<Value>(Ok(json!({ "total": "12.50" })));
        assert_eq!(r.is_error, Some(false));
        assert_eq!(r.structured_content, Some(json!({ "total": "12.50" })));
        assert_eq!(text(&r), json!({ "total": "12.50" }));
    }

    #[test]
    fn arrays_are_wrapped_in_an_object() {
        let r = respond::<Value>(Ok(json!([1, 2])));
        assert_eq!(r.structured_content, Some(json!({ "items": [1, 2] })));
    }

    #[test]
    fn errors_are_the_rest_body() {
        let mut fields = FieldErrors::new();
        fields.add("lines.0.vatRate", "invalid");
        let r = respond::<Value>(Err(AppError::Validation(fields)));
        assert_eq!(r.is_error, Some(true));
        let body = json!({ "code": "validation", "fields": { "lines.0.vatRate": "invalid" } });
        assert_eq!(text(&r), body);
        assert_eq!(r.structured_content, Some(body));
    }

    #[test]
    fn internal_errors_hide_the_cause() {
        let r = respond::<Value>(Err(AppError::Internal(anyhow::anyhow!("secret dsn"))));
        assert_eq!(text(&r), json!({ "code": "internal" }));
    }

    #[test]
    fn with_sets_a_key_on_objects_only() {
        assert_eq!(with(json!({ "a": 1 }), "b", 2), json!({ "a": 1, "b": 2 }));
        assert_eq!(with(json!([1]), "b", 2), json!([1]));
    }
}
