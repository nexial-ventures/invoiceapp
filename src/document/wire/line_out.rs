//! Response shape of document lines (positions, computed bases).

use rust_decimal::Decimal;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::document::compute;
use crate::document::line::LineData;
use crate::document::subtotals::Resolved;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemLine {
    pub position: i32,
    pub description: String,
    pub quantity: Decimal,
    pub unit: Option<String>,
    pub unit_price: Decimal,
    pub discount_pct: Decimal,
    pub vat_rate: Decimal,
    pub base: Decimal,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TextLine {
    pub position: i32,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubtotalLine {
    pub position: i32,
    pub description: String,
    pub refs: Vec<i32>,
    pub collapse: bool,
    /// Sum of the referenced bases.
    pub base: Decimal,
    /// The shared rate of the members.
    pub vat_rate: Decimal,
}

/// One deducted rate; negative amounts.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdvanceRecap {
    pub vat_rate: Decimal,
    pub base: Decimal,
    pub vat: Decimal,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdvanceLine {
    pub position: i32,
    pub advance_document_id: Uuid,
    /// Server-generated, e.g. "Odpočet zálohy DP20260003".
    pub description: String,
    /// Negative: minus the deducted bases.
    pub base: Decimal,
    pub recap: Vec<AdvanceRecap>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Line {
    Item(ItemLine),
    Text(TextLine),
    Subtotal(SubtotalLine),
    Advance(AdvanceLine),
}

/// `-x` with 2 dp, never a "-0.00".
fn negated(x: Decimal) -> Decimal {
    compute::round2(Decimal::ZERO.saturating_sub(x))
}

/// Response lines with positions and computed bases.
pub fn lines_out(lines: &[LineData], resolved: &Resolved) -> Vec<Line> {
    lines
        .iter()
        .zip(resolved)
        .zip(1..)
        .map(|((line, r), position)| {
            let (vat_rate, base) = r.unwrap_or_default();
            match line {
                LineData::Item(i) => Line::Item(ItemLine {
                    position,
                    description: i.description.clone(),
                    quantity: i.quantity.normalize(),
                    unit: i.unit.clone(),
                    unit_price: i.unit_price.normalize(),
                    discount_pct: i.discount_pct.normalize(),
                    vat_rate: i.vat_rate.normalize(),
                    base: compute::round2(base),
                }),
                LineData::Text { description } => Line::Text(TextLine {
                    position,
                    description: description.clone(),
                }),
                LineData::Subtotal {
                    description,
                    refs,
                    collapse,
                } => Line::Subtotal(SubtotalLine {
                    position,
                    description: description.clone(),
                    refs: refs.clone(),
                    collapse: *collapse,
                    base: compute::round2(base),
                    vat_rate: vat_rate.normalize(),
                }),
                LineData::Advance(a) => Line::Advance(AdvanceLine {
                    position,
                    advance_document_id: a.document_id,
                    description: a.description.clone(),
                    base: negated(
                        a.recap
                            .iter()
                            .fold(Decimal::ZERO, |acc, r| acc.saturating_add(r.base)),
                    ),
                    recap: a
                        .recap
                        .iter()
                        .map(|r| AdvanceRecap {
                            vat_rate: r.vat_rate.normalize(),
                            base: negated(r.base),
                            vat: negated(r.vat),
                        })
                        .collect(),
                }),
            }
        })
        .collect()
}
