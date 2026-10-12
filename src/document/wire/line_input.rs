//! Wire shape of a document line in requests and its validation.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use uuid::Uuid;

use crate::document::line::{AdvanceData, ItemData, LineData, MAX_INPUT, VatMode};
use crate::error::FieldErrors;
use crate::num_parse::{decimal, parse_rate};
use crate::validation::{self as v, Check};

pub const MAX_LINES: usize = 1000;

/// One request line. `kind` selects which fields apply:
/// `item` (description, quantity, unit, unitPrice, discountPct, vatRate),
/// `text` (description), `subtotal` (description, refs, collapse),
/// `advance` (advanceDocumentId; description and amounts come from the server).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct LineInput {
    /// `item` | `text` | `subtotal` | `advance`.
    pub kind: String,
    pub description: String,
    /// Decimal string, at most 4 dp, non-zero.
    #[serde(deserialize_with = "crate::num_text::opt_decimal")]
    #[schemars(with = "Option<crate::num_text::DecimalText>")]
    pub quantity: Option<String>,
    pub unit: Option<String>,
    /// Excl. VAT, decimal string, at most 4 dp, may be negative.
    #[serde(deserialize_with = "crate::num_text::opt_decimal")]
    #[schemars(with = "Option<crate::num_text::DecimalText>")]
    pub unit_price: Option<String>,
    /// `"0"`..`"100"`, at most 2 dp; default `"0"`.
    #[serde(deserialize_with = "crate::num_text::opt_decimal")]
    #[schemars(with = "Option<crate::num_text::DecimalText>")]
    pub discount_pct: Option<String>,
    /// Percent; default = the default VAT rate (`"0"` for `non_payer`).
    #[serde(deserialize_with = "crate::num_text::opt_decimal")]
    #[schemars(with = "Option<crate::num_text::DecimalText>")]
    pub vat_rate: Option<String>,
    /// 1-based positions of the lines a subtotal sums.
    pub refs: Vec<i32>,
    pub collapse: bool,
    /// `advance`: the DDPP (or settled proforma) to deduct.
    pub advance_document_id: Option<Uuid>,
}

fn required_decimal(s: Option<&str>, dp: u32) -> Check<Decimal> {
    decimal(s.ok_or("required")?, dp, MAX_INPUT)
}

/// Whether descriptions are checked: saving does, the live compute endpoint
/// only validates what affects amounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Texts {
    Validate,
    Skip,
}

/// Validate every line; errors go to `lines.<index>.<field>`.
/// `default_rate` fills a missing item `vatRate`.
pub fn validate_lines(
    lines: Vec<LineInput>,
    vat_mode: Option<VatMode>,
    default_rate: Option<Decimal>,
    texts: Texts,
    e: &mut FieldErrors,
) -> Vec<LineData> {
    if lines.len() > MAX_LINES {
        e.add("lines", "too_long");
        return Vec::new();
    }
    let default_rate = match vat_mode {
        Some(VatMode::NonPayer) => Some(Decimal::ZERO),
        _ => default_rate,
    };
    lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| validate_line(i, l, default_rate, texts, e))
        .collect()
}

fn validate_line(
    i: usize,
    l: LineInput,
    default_rate: Option<Decimal>,
    texts: Texts,
    e: &mut FieldErrors,
) -> LineData {
    let f = |name: &str| format!("lines.{i}.{name}");
    let text = |e: &mut FieldErrors, name: &str, value: Check<String>, raw: &str| match texts {
        Texts::Validate => e.check(&f(name), value).unwrap_or_default(),
        Texts::Skip => raw.trim().to_string(),
    };
    match l.kind.as_str() {
        "item" => {
            let quantity = e
                .check(
                    &f("quantity"),
                    required_decimal(l.quantity.as_deref(), 4)
                        .and_then(|q| if q.is_zero() { Err("invalid") } else { Ok(q) }),
                )
                .unwrap_or_default();
            let discount_pct = match l.discount_pct.as_deref().map(str::trim) {
                None | Some("") => Some(Decimal::ZERO),
                Some(s) => e.check(&f("discountPct"), parse_rate(s)),
            };
            let vat_rate = match l.vat_rate.as_deref().map(str::trim) {
                None | Some("") => default_rate.ok_or("required"),
                Some(s) => parse_rate(s),
            };
            let unit = match texts {
                Texts::Validate => e
                    .check(&f("unit"), v::opt_text(l.unit.as_deref(), 20))
                    .flatten(),
                Texts::Skip => v::opt_text(l.unit.as_deref(), usize::MAX).ok().flatten(),
            };
            LineData::Item(ItemData {
                description: text(
                    e,
                    "description",
                    v::required_text(&l.description, 500),
                    &l.description,
                ),
                quantity,
                unit,
                unit_price: e
                    .check(
                        &f("unitPrice"),
                        required_decimal(l.unit_price.as_deref(), 4),
                    )
                    .unwrap_or_default(),
                discount_pct: discount_pct.unwrap_or_default(),
                vat_rate: e.check(&f("vatRate"), vat_rate).unwrap_or_default(),
            })
        }
        "text" => LineData::Text {
            description: text(
                e,
                "description",
                v::required_text(&l.description, 500),
                &l.description,
            ),
        },
        "subtotal" => LineData::Subtotal {
            description: text(
                e,
                "description",
                v::text(&l.description, 500),
                &l.description,
            ),
            refs: l.refs,
            collapse: l.collapse,
        },
        "advance" => LineData::Advance(AdvanceData {
            document_id: e
                .check(
                    &f("advanceDocumentId"),
                    l.advance_document_id.ok_or("required"),
                )
                .unwrap_or_default(),
            description: String::new(),
            recap: Vec::new(),
        }),
        _ => {
            e.add(&f("kind"), "invalid");
            LineData::Text {
                description: String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(qty: &str, price: &str) -> LineInput {
        LineInput {
            kind: "item".into(),
            description: "Work".into(),
            quantity: Some(qty.into()),
            unit_price: Some(price.into()),
            ..Default::default()
        }
    }

    #[test]
    fn defaults_rate_and_discount() {
        let mut e = FieldErrors::new();
        let rate: Decimal = "21".parse().expect("d");
        let lines = validate_lines(
            vec![item("2", "-10")],
            Some(VatMode::Standard),
            Some(rate),
            Texts::Validate,
            &mut e,
        );
        assert!(e.is_empty(), "{e:?}");
        let LineData::Item(it) = &lines[0] else {
            panic!("item expected")
        };
        assert_eq!((it.vat_rate, it.discount_pct), (rate, Decimal::ZERO));
        let lines = validate_lines(
            vec![item("2", "10")],
            Some(VatMode::NonPayer),
            Some(rate),
            Texts::Validate,
            &mut e,
        );
        let LineData::Item(it) = &lines[0] else {
            panic!("item expected")
        };
        assert_eq!(it.vat_rate, Decimal::ZERO);
    }

    #[test]
    fn reports_errors_by_index() {
        let mut e = FieldErrors::new();
        let bad = LineInput {
            discount_pct: Some("101".into()),
            vat_rate: Some("x".into()),
            ..item("0", "abc")
        };
        let lines = vec![
            item("1", "1"),
            bad,
            LineInput {
                kind: "text".into(),
                ..Default::default()
            },
            LineInput {
                kind: "bogus".into(),
                ..Default::default()
            },
        ];
        validate_lines(
            lines.clone(),
            Some(VatMode::Standard),
            None,
            Texts::Validate,
            &mut e,
        );
        let mut expected = FieldErrors::new();
        for (field, reason) in [
            ("lines.0.vatRate", "required"),
            ("lines.1.quantity", "invalid"),
            ("lines.1.unitPrice", "invalid"),
            ("lines.1.discountPct", "invalid"),
            ("lines.1.vatRate", "invalid"),
            ("lines.2.description", "required"),
            ("lines.3.kind", "invalid"),
        ] {
            expected.add(field, reason);
        }
        assert_eq!(e, expected);

        // Compute skips descriptions; amount errors stay.
        let mut e = FieldErrors::new();
        validate_lines(lines, Some(VatMode::Standard), None, Texts::Skip, &mut e);
        let mut expected = FieldErrors::new();
        for (field, reason) in [
            ("lines.0.vatRate", "required"),
            ("lines.1.quantity", "invalid"),
            ("lines.1.unitPrice", "invalid"),
            ("lines.1.discountPct", "invalid"),
            ("lines.1.vatRate", "invalid"),
            ("lines.3.kind", "invalid"),
        ] {
            expected.add(field, reason);
        }
        assert_eq!(e, expected);
    }

    #[test]
    fn too_many_lines() {
        let mut e = FieldErrors::new();
        validate_lines(
            vec![item("1", "1"); MAX_LINES + 1],
            None,
            None,
            Texts::Validate,
            &mut e,
        );
        let mut expected = FieldErrors::new();
        expected.add("lines", "too_long");
        assert_eq!(e, expected);
    }
}
