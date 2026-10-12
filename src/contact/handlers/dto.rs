use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::contact::entity::contact;
use crate::error::{AppError, FieldErrors};
use crate::validation as v;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub id: Uuid,
    pub name: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub street: String,
    pub city: String,
    pub zip: String,
    pub country: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub note: Option<String>,
    pub default_due_days: Option<i32>,
    pub default_locale: Option<String>,
    pub default_currency: Option<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

impl From<contact::Model> for Contact {
    fn from(m: contact::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            ico: m.ico,
            dic: m.dic,
            street: m.street,
            city: m.city,
            zip: m.zip,
            country: m.country,
            email: m.email,
            phone: m.phone,
            note: m.note,
            default_due_days: m.default_due_days,
            default_locale: m.default_locale,
            default_currency: m.default_currency,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Create/update body (PUT replaces every field).
#[derive(Debug, Clone, Default, Deserialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ContactInput {
    pub name: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub street: String,
    pub city: String,
    pub zip: String,
    /// ISO 3166 alpha-2; empty → `CZ`.
    pub country: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub note: Option<String>,
    pub default_due_days: Option<i32>,
    /// `cs` | `en` | null.
    pub default_locale: Option<String>,
    pub default_currency: Option<String>,
}

impl ContactInput {
    pub fn validate(self) -> Result<Self, AppError> {
        let mut e = FieldErrors::new();
        let out = Self {
            name: e
                .check("name", v::required_text(&self.name, 200))
                .unwrap_or_default(),
            ico: e.check("ico", v::opt_ico(self.ico.as_deref())).flatten(),
            dic: e.check("dic", v::opt_dic(self.dic.as_deref())).flatten(),
            street: e
                .check("street", v::text(&self.street, 200))
                .unwrap_or_default(),
            city: e
                .check("city", v::text(&self.city, 100))
                .unwrap_or_default(),
            zip: e.check("zip", v::text(&self.zip, 20)).unwrap_or_default(),
            country: e
                .check("country", v::country(&self.country))
                .unwrap_or_default(),
            email: e
                .check("email", v::opt_email(self.email.as_deref()))
                .flatten(),
            phone: e
                .check("phone", v::opt_text(self.phone.as_deref(), 50))
                .flatten(),
            note: e
                .check("note", v::opt_text(self.note.as_deref(), 2000))
                .flatten(),
            default_due_days: e
                .check(
                    "defaultDueDays",
                    self.default_due_days.map(v::due_days).transpose(),
                )
                .flatten(),
            default_locale: e
                .check(
                    "defaultLocale",
                    v::opt_locale(self.default_locale.as_deref()),
                )
                .flatten(),
            default_currency: e
                .check(
                    "defaultCurrency",
                    v::opt_currency(self.default_currency.as_deref()),
                )
                .flatten(),
        };
        e.into_result()?;
        Ok(out)
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ContactList {
    pub items: Vec<Contact>,
    pub total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_collects_all_fields() {
        let err = ContactInput {
            ico: Some("12345678".into()),
            default_due_days: Some(400),
            default_locale: Some("de".into()),
            default_currency: Some("EURO".into()),
            email: Some("nope".into()),
            ..Default::default()
        }
        .validate()
        .expect_err("invalid");
        let AppError::Validation(fields) = err else {
            panic!("expected validation error");
        };
        let mut expected = FieldErrors::new();
        for (f, r) in [
            ("name", "required"),
            ("ico", "invalid_ico"),
            ("defaultDueDays", "invalid"),
            ("defaultLocale", "invalid"),
            ("defaultCurrency", "invalid"),
            ("email", "invalid"),
        ] {
            expected.add(f, r);
        }
        assert_eq!(fields, expected);
    }

    #[test]
    fn validation_normalizes() {
        let c = ContactInput {
            name: " ACME s.r.o. ".into(),
            ico: Some("270 74 358".into()),
            dic: Some("cz27074358".into()),
            country: "".into(),
            default_currency: Some("eur".into()),
            note: Some("  ".into()),
            ..Default::default()
        }
        .validate()
        .expect("valid");
        assert_eq!(c.name, "ACME s.r.o.");
        assert_eq!(c.ico.as_deref(), Some("27074358"));
        assert_eq!(c.dic.as_deref(), Some("CZ27074358"));
        assert_eq!(c.country, "CZ");
        assert_eq!(c.default_currency.as_deref(), Some("EUR"));
        assert_eq!(c.note, None);
    }
}
