//! Tool arguments: parsed here (not by rmcp's `Parameters`) so a shape error is
//! the REST validation body `{"code":"validation","fields":{…}}` instead of
//! rmcp's plain text.

use std::sync::Arc;

use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_path_to_error::Segment;

use crate::error::AppError;

/// The advertised input schema of `T` (cached by rmcp per type).
pub fn input<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    rmcp::handler::server::common::schema_for_type::<T>()
}

/// `args` as `T`; the first failing field → `fields.<path>` = `required`
/// (missing) or `invalid` (wrong type / format). Paths use the REST form
/// `lines.0.quantity`.
pub fn parse<T: DeserializeOwned>(args: JsonObject) -> Result<T, AppError> {
    serde_path_to_error::deserialize(Value::Object(args)).map_err(|e| {
        let mut path: Vec<String> = e
            .path()
            .iter()
            .filter_map(|s| match s {
                Segment::Map { key } => Some(key.clone()),
                Segment::Seq { index } => Some(index.to_string()),
                Segment::Enum { .. } | Segment::Unknown => None,
            })
            .collect();
        let message = e.inner().to_string();
        let reason = match missing_field(&message) {
            Some(field) => {
                path.push(field.to_string());
                "required"
            }
            None => "invalid",
        };
        let field = if path.is_empty() {
            "arguments".to_string()
        } else {
            path.join(".")
        };
        AppError::field(&field, reason)
    })
}

/// serde's derive reports a missing field only in its message.
fn missing_field(message: &str) -> Option<&str> {
    message.strip_prefix("missing field `")?.split('`').next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::wire::input::DocumentInput;
    use crate::mcp::params::AddPayment;
    use serde_json::json;

    fn fields(args: Value) -> Option<(String, &'static str)> {
        let Value::Object(obj) = args else {
            panic!("object")
        };
        match parse::<AddPayment>(obj) {
            Err(AppError::Validation(f)) => {
                let field = ["id", "date", "amount", "note", "exchangeRate"]
                    .into_iter()
                    .find(|k| f.get(k).is_some())?;
                Some((field.to_string(), f.get(field)?))
            }
            _ => None,
        }
    }

    fn doc_error(args: Value) -> AppError {
        let Value::Object(obj) = args else {
            panic!("object")
        };
        parse::<DocumentInput>(obj).expect_err("invalid")
    }

    #[test]
    fn missing_and_invalid_fields() {
        let id = "00000000-0000-4000-8000-000000000001";
        assert_eq!(
            fields(json!({ "id": id, "amount": "1" })),
            Some(("date".into(), "required"))
        );
        assert_eq!(
            fields(json!({ "id": "x", "date": "2026-10-01", "amount": "1" })),
            Some(("id".into(), "invalid"))
        );
        assert_eq!(
            fields(json!({ "id": id, "date": "1.10.2026", "amount": "1" })),
            Some(("date".into(), "invalid"))
        );
    }

    #[test]
    fn nested_paths_use_the_rest_form() {
        let err = doc_error(json!({ "lines": [{ "kind": "item" }, { "quantity": true }] }));
        let AppError::Validation(f) = err else {
            panic!("validation")
        };
        assert_eq!(f.get("lines.1.quantity"), Some("invalid"));
    }

    #[test]
    fn decimal_fields_take_numbers() {
        let Value::Object(obj) = json!({ "lines": [{ "kind": "item", "quantity": 2,
            "unitPrice": 1234.56, "discountPct": 0.1, "vatRate": "21" }] })
        else {
            panic!("object")
        };
        let doc: DocumentInput = parse(obj).expect("valid");
        let l = &doc.lines[0];
        assert_eq!(l.quantity.as_deref(), Some("2"));
        assert_eq!(l.unit_price.as_deref(), Some("1234.56"));
        assert_eq!(l.discount_pct.as_deref(), Some("0.1"));
        assert_eq!(l.vat_rate.as_deref(), Some("21"));
    }

    #[test]
    fn missing_field_parses_the_serde_message() {
        assert_eq!(missing_field("missing field `date`"), Some("date"));
        assert_eq!(missing_field("invalid type: integer"), None);
    }
}
