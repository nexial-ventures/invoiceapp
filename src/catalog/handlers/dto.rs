//! Catalog wire types and their validation.

use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::catalog::entity::item;
use crate::currency::LOCAL_CURRENCY;
use crate::document::line::MAX_INPUT;
use crate::error::{AppError, FieldErrors};
use crate::num_parse::decimal;
use crate::num_parse::parse_rate;
use crate::validation as v;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogItem {
    pub id: Uuid,
    pub name: String,
    pub unit: Option<String>,
    /// Excl. VAT, at most 4 dp.
    pub unit_price: Decimal,
    pub currency: String,
    pub vat_rate: Decimal,
    pub active: bool,
    pub note: Option<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

impl From<item::Model> for CatalogItem {
    fn from(m: item::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            unit: m.unit,
            unit_price: m.unit_price.normalize(),
            currency: m.currency,
            vat_rate: m.vat_rate.normalize(),
            active: m.active,
            note: m.note,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogItemInput {
    /// Required, at most 200 characters.
    pub name: String,
    pub unit: Option<String>,
    /// Decimal string, at most 4 dp, may be negative.
    pub unit_price: Option<String>,
    /// ISO 4217; default `CZK`.
    pub currency: Option<String>,
    pub vat_rate: Option<String>,
    /// Default `true`.
    pub active: Option<bool>,
    pub note: Option<String>,
}

/// Validated [`CatalogItemInput`].
#[derive(Debug, Clone, PartialEq)]
pub struct ItemData {
    pub name: String,
    pub unit: Option<String>,
    pub unit_price: Decimal,
    pub currency: String,
    pub vat_rate: Decimal,
    pub active: bool,
    pub note: Option<String>,
}

impl CatalogItemInput {
    pub fn validate(self) -> Result<ItemData, AppError> {
        let mut e = FieldErrors::new();
        let data = ItemData {
            name: e
                .check("name", v::required_text(&self.name, 200))
                .unwrap_or_default(),
            unit: e
                .check("unit", v::opt_text(self.unit.as_deref(), 20))
                .flatten(),
            unit_price: e
                .check(
                    "unitPrice",
                    decimal(self.unit_price.as_deref().unwrap_or_default(), 4, MAX_INPUT),
                )
                .unwrap_or_default(),
            currency: e
                .check(
                    "currency",
                    v::currency(self.currency.as_deref().unwrap_or(LOCAL_CURRENCY)),
                )
                .unwrap_or_default(),
            vat_rate: e
                .check(
                    "vatRate",
                    parse_rate(self.vat_rate.as_deref().unwrap_or_default()),
                )
                .unwrap_or_default(),
            active: self.active.unwrap_or(true),
            note: e
                .check("note", v::opt_text(self.note.as_deref(), 2000))
                .flatten(),
        };
        e.into_result()?;
        Ok(data)
    }
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct ItemQuery {
    /// Case-insensitive substring of the name.
    pub q: Option<String>,
    /// Only active (`true`) or inactive (`false`) items.
    pub active: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GroupQuery {
    /// Case-insensitive substring of the name.
    pub q: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMember {
    pub item_id: Uuid,
    pub quantity: Decimal,
    /// 1-based order within the group.
    pub position: i32,
    pub item: CatalogItem,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogGroup {
    pub id: Uuid,
    pub name: String,
    pub collapse: bool,
    pub members: Vec<GroupMember>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct MemberInput {
    pub item_id: Option<Uuid>,
    /// Decimal string, at most 4 dp, non-zero.
    pub quantity: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogGroupInput {
    /// Required, at most 200 characters (the inserted subtotal's description).
    pub name: String,
    /// Default `true`.
    pub collapse: Option<bool>,
    /// Order = position; at least one, unique `itemId`, one shared VAT rate.
    pub members: Vec<MemberInput>,
}

/// Validated [`CatalogGroupInput`]: members as `(itemId, quantity)`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupData {
    pub name: String,
    pub collapse: bool,
    pub members: Vec<(Uuid, Decimal)>,
}

/// More members than any sensible group; keeps the request bounded.
const MAX_MEMBERS: usize = 1000;

impl CatalogGroupInput {
    /// Field checks; whether the items exist and share a rate is
    /// [`check_items`]'s job.
    pub fn validate(self) -> Result<GroupData, AppError> {
        let mut e = FieldErrors::new();
        let name = e
            .check("name", v::required_text(&self.name, 200))
            .unwrap_or_default();
        if self.members.is_empty() {
            e.add("members", "required");
        } else if self.members.len() > MAX_MEMBERS {
            e.add("members", "too_long");
        }
        let mut members = Vec::with_capacity(self.members.len());
        for (i, m) in self.members.into_iter().enumerate() {
            let item_id = e.check(&format!("members.{i}.itemId"), m.item_id.ok_or("required"));
            let quantity = e.check(
                &format!("members.{i}.quantity"),
                decimal(m.quantity.as_deref().unwrap_or_default(), 4, MAX_INPUT)
                    .and_then(|q| if q.is_zero() { Err("invalid") } else { Ok(q) }),
            );
            if let Some(id) = item_id {
                if members.iter().any(|(other, _)| *other == id) {
                    e.add(&format!("members.{i}.itemId"), "duplicate");
                }
                members.push((id, quantity.unwrap_or_default()));
            }
        }
        e.into_result()?;
        Ok(GroupData {
            name,
            collapse: self.collapse.unwrap_or(true),
            members,
        })
    }
}

/// Every member must name an existing item (`members.N.itemId: invalid`),
/// and all members must share one VAT rate (`members: mixed_vat`) — a group
/// is inserted as lines under one subtotal.
pub fn check_items(members: &[(Uuid, Decimal)], items: &HashMap<Uuid, item::Model>) -> FieldErrors {
    let mut e = FieldErrors::new();
    let mut rate: Option<Decimal> = None;
    for (i, (id, _)) in members.iter().enumerate() {
        match items.get(id) {
            None => e.add(&format!("members.{i}.itemId"), "invalid"),
            Some(it) => match rate {
                Some(r) if r != it.vat_rate => e.add("members", "mixed_vat"),
                _ => rate = Some(it.vat_rate),
            },
        }
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    fn fields(err: AppError) -> FieldErrors {
        match err {
            AppError::Validation(f) => f,
            other => panic!("expected validation, got {other:?}"),
        }
    }

    fn expected(pairs: &[(&str, &'static str)]) -> FieldErrors {
        let mut e = FieldErrors::new();
        for (f, r) in pairs {
            e.add(f, r);
        }
        e
    }

    #[test]
    fn item_defaults_and_errors() {
        let ok = CatalogItemInput {
            name: " Konzultace ".into(),
            unit_price: Some("1500.50".into()),
            vat_rate: Some("21".into()),
            ..Default::default()
        }
        .validate()
        .expect("valid");
        assert_eq!(ok.name, "Konzultace");
        assert_eq!((ok.currency.as_str(), ok.active), ("CZK", true));
        assert_eq!(ok.unit_price, d("1500.5"));

        let err = CatalogItemInput {
            unit_price: Some("1.23456".into()),
            currency: Some("euro".into()),
            vat_rate: Some("101".into()),
            ..Default::default()
        }
        .validate()
        .expect_err("invalid");
        assert_eq!(
            fields(err),
            expected(&[
                ("name", "required"),
                ("unitPrice", "invalid"),
                ("currency", "invalid"),
                ("vatRate", "invalid"),
            ])
        );
    }

    #[test]
    fn group_member_rules() {
        let a = Uuid::from_u128(1);
        let m = |id: Option<Uuid>, q: &str| MemberInput {
            item_id: id,
            quantity: Some(q.into()),
        };
        let err = CatalogGroupInput {
            name: "Balíček".into(),
            collapse: None,
            members: vec![m(Some(a), "1"), m(Some(a), "2"), m(None, "0")],
        }
        .validate()
        .expect_err("invalid");
        assert_eq!(
            fields(err),
            expected(&[
                ("members.1.itemId", "duplicate"),
                ("members.2.itemId", "required"),
                ("members.2.quantity", "invalid"),
            ])
        );
        let err = CatalogGroupInput {
            name: "x".into(),
            ..Default::default()
        }
        .validate()
        .expect_err("empty");
        assert_eq!(fields(err), expected(&[("members", "required")]));
        let ok = CatalogGroupInput {
            name: "x".into(),
            collapse: None,
            members: vec![m(Some(a), "2.5")],
        }
        .validate()
        .expect("valid");
        assert!(ok.collapse);
        assert_eq!(ok.members, vec![(a, d("2.5"))]);
    }

    #[test]
    fn items_must_exist_and_share_a_rate() {
        let item = |id: u128, rate: &str| item::Model {
            id: Uuid::from_u128(id),
            space_id: Uuid::nil(),
            name: "i".into(),
            unit: None,
            unit_price: d("1"),
            currency: "CZK".into(),
            vat_rate: d(rate),
            active: true,
            note: None,
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        };
        let items: HashMap<_, _> = [item(1, "21"), item(2, "21"), item(3, "12")]
            .into_iter()
            .map(|i| (i.id, i))
            .collect();
        let m = |id: u128| (Uuid::from_u128(id), d("1"));
        assert!(check_items(&[m(1), m(2)], &items).is_empty());
        assert_eq!(
            check_items(&[m(1), m(3), m(9)], &items),
            expected(&[("members", "mixed_vat"), ("members.2.itemId", "invalid")])
        );
    }
}
