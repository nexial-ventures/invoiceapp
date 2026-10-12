//! Validated document lines and the enumerated document fields, shared by the
//! pure computation, the repos and the handlers.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `10^exp` (`exp <= 19`) as a `Decimal`, usable in consts.
const fn pow10(exp: u32) -> Decimal {
    let n = 10u64.pow(exp);
    Decimal::from_parts(n as u32, (n >> 32) as u32, 0, false, 0)
}

/// Money columns are `numeric(18,2)`: they hold `|x| < 10^16`.
pub const MAX_AMOUNT: Decimal = pow10(16);
/// Quantities and unit prices (`numeric(18,4)`) are capped well below the
/// column range so `quantity × unitPrice × 100` stays far inside `Decimal`.
pub const MAX_INPUT: Decimal = pow10(12);
/// Exchange rates are `numeric(18,6)`: they hold `|x| < 10^12`.
pub const MAX_RATE: Decimal = pow10(12);

#[derive(Debug, Clone, PartialEq)]
pub struct ItemData {
    pub description: String,
    pub quantity: Decimal,
    pub unit: Option<String>,
    pub unit_price: Decimal,
    pub discount_pct: Decimal,
    pub vat_rate: Decimal,
}

/// One VAT rate of a deducted advance; amounts are the **positive** deducted
/// values (stored as JSON on the line, negated in responses and totals).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvanceRow {
    pub vat_rate: Decimal,
    pub base: Decimal,
    pub vat: Decimal,
    /// The advance document's own CZK amounts; `None` → converted at the
    /// deducting document's rate.
    pub base_czk: Option<Decimal>,
    pub vat_czk: Option<Decimal>,
}

/// An `advance` line: deducts an issued DDPP (or, non-payer form, the paid
/// amount of the settled proforma). `description` and `recap` are filled by
/// the server from the referenced document.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvanceData {
    pub document_id: Uuid,
    pub description: String,
    pub recap: Vec<AdvanceRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LineData {
    Item(ItemData),
    Text {
        description: String,
    },
    /// `refs` are 1-based positions of other lines.
    Subtotal {
        description: String,
        refs: Vec<i32>,
        collapse: bool,
    },
    Advance(AdvanceData),
}

impl LineData {
    pub fn kind(&self) -> &'static str {
        match self {
            LineData::Item(_) => "item",
            LineData::Text { .. } => "text",
            LineData::Subtotal { .. } => "subtotal",
            LineData::Advance(_) => "advance",
        }
    }
}

/// Declares a string-backed enum with `as_str` / `parse` (wire = DB value),
/// usable directly as a `text` column (CHECK-constrained to the same values).
macro_rules! str_enum {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
            schemars::JsonSchema, sea_orm::EnumIter, sea_orm::DeriveActiveEnum,
        )]
        #[serde(rename_all = "snake_case")]
        #[sea_orm(rs_type = "String", db_type = "Text")]
        pub enum $name { $(#[sea_orm(string_value = $s)] $variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s),+ }
            }

            pub fn parse(s: &str) -> Option<$name> {
                Self::ALL.iter().copied().find(|v| v.as_str() == s)
            }
        }
    };
}

str_enum!(Direction {
    Issued => "issued",
    Received => "received",
});

str_enum!(VatMode {
    Standard => "standard",
    ReverseCharge => "reverse_charge",
    Exempt => "exempt",
    NonPayer => "non_payer",
});

impl VatMode {
    /// Only `standard` charges VAT; the other modes keep the rates for display.
    pub fn charges_vat(self) -> bool {
        self == VatMode::Standard
    }
}

str_enum!(Status {
    Draft => "draft",
    Issued => "issued",
    Cancelled => "cancelled",
});

str_enum!(PaymentMethod {
    BankTransfer => "bank_transfer",
    Cash => "cash",
    Card => "card",
    Other => "other",
});

str_enum!(RoundingMode {
    None => "none",
    Half => "half",
    Unit => "unit",
});

str_enum!(PaymentState {
    Unpaid => "unpaid",
    Partial => "partial",
    Paid => "paid",
    Overpaid => "overpaid",
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits() {
        assert_eq!(
            MAX_AMOUNT,
            "10000000000000000".parse::<Decimal>().expect("d")
        );
        assert_eq!(MAX_INPUT, Decimal::from(1_000_000_000_000_i64));
        assert_eq!(MAX_RATE, MAX_INPUT);
    }

    #[test]
    fn enums_round_trip() {
        for m in VatMode::ALL {
            assert_eq!(VatMode::parse(m.as_str()), Some(*m));
            assert_eq!(
                serde_json::to_value(m).expect("serialize"),
                serde_json::json!(m.as_str())
            );
        }
        assert_eq!(
            PaymentMethod::parse("bank_transfer"),
            Some(PaymentMethod::BankTransfer)
        );
        assert_eq!(Status::parse("sent"), None);
        assert!(VatMode::Standard.charges_vat());
        assert!(!VatMode::NonPayer.charges_vat());
    }

    /// The column value is the wire value: stored rows and the CHECK
    /// constraints keep the strings they always had.
    #[test]
    fn database_values_are_the_wire_values() {
        use sea_orm::ActiveEnum;
        fn check<T: ActiveEnum<Value = String> + serde::Serialize + PartialEq + std::fmt::Debug>(
            all: &[T],
        ) {
            for v in all {
                let db = v.to_value();
                assert_eq!(
                    serde_json::to_value(v).expect("serialize"),
                    serde_json::json!(db)
                );
                assert_eq!(&T::try_from_value(&db).expect("parse"), v);
            }
        }
        check(Direction::ALL);
        check(VatMode::ALL);
        check(Status::ALL);
        check(PaymentMethod::ALL);
        check(RoundingMode::ALL);
        check(PaymentState::ALL);
        assert_eq!(Direction::Received.to_value(), "received");
        assert!(RoundingMode::try_from_value(&"whole".to_string()).is_err());
    }
}
