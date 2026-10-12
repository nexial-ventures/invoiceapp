//! The company's document defaults (`docs/api/defaults.md`): payment method,
//! currency, rounding per method and the notes per locale.

use sea_orm::ActiveValue::Set;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::document::line::{PaymentMethod, RoundingMode};
use crate::document::rounding;
use crate::error::FieldErrors;
use crate::settings::entity::company;
use crate::validation as v;

const NOTE_MAX: usize = 2000;

/// A note per document locale.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct LocaleNotes {
    pub cs: Option<String>,
    pub en: Option<String>,
}

/// The stored defaults (response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocDefaults {
    /// Rounding of new documents paid in cash.
    pub default_rounding_cash: RoundingMode,
    /// Rounding of new documents paid by any other method.
    pub default_rounding_other: RoundingMode,
    pub default_payment_method: PaymentMethod,
    /// ISO 4217; the contact's default currency wins.
    pub default_currency: String,
    pub default_header_note: LocaleNotes,
    pub default_footer_note: LocaleNotes,
}

/// `PUT` body part: every field optional, absent (or `null`) = keep the
/// stored value; a present note object replaces both locales.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DocDefaultsInput {
    /// `none` | `half` | `unit`.
    pub default_rounding_cash: Option<String>,
    /// `none` | `half` | `unit`.
    pub default_rounding_other: Option<String>,
    /// `bank_transfer` | `cash` | `card` | `other`.
    pub default_payment_method: Option<String>,
    pub default_currency: Option<String>,
    pub default_header_note: Option<LocaleNotes>,
    pub default_footer_note: Option<LocaleNotes>,
}

/// Validated [`DocDefaultsInput`]: `None` = keep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultsUpdate {
    pub rounding_cash: Option<RoundingMode>,
    pub rounding_other: Option<RoundingMode>,
    pub payment_method: Option<PaymentMethod>,
    pub currency: Option<String>,
    pub header_note: Option<LocaleNotes>,
    pub footer_note: Option<LocaleNotes>,
}

fn notes(n: Option<LocaleNotes>, field: &str, e: &mut FieldErrors) -> Option<LocaleNotes> {
    n.map(|n| LocaleNotes {
        cs: e
            .check(
                &format!("{field}.cs"),
                v::opt_text(n.cs.as_deref(), NOTE_MAX),
            )
            .flatten(),
        en: e
            .check(
                &format!("{field}.en"),
                v::opt_text(n.en.as_deref(), NOTE_MAX),
            )
            .flatten(),
    })
}

impl DocDefaultsInput {
    pub fn validate(self, e: &mut FieldErrors) -> DefaultsUpdate {
        let mode = |s: Option<String>, field: &str, e: &mut FieldErrors| {
            s.and_then(|s| e.check(field, rounding::parse(&s)))
        };
        DefaultsUpdate {
            rounding_cash: mode(self.default_rounding_cash, "defaultRoundingCash", e),
            rounding_other: mode(self.default_rounding_other, "defaultRoundingOther", e),
            payment_method: self.default_payment_method.and_then(|s| {
                e.check(
                    "defaultPaymentMethod",
                    PaymentMethod::parse(s.trim()).ok_or("invalid"),
                )
            }),
            currency: self
                .default_currency
                .and_then(|s| e.check("defaultCurrency", v::currency(&s))),
            header_note: notes(self.default_header_note, "defaultHeaderNote", e),
            footer_note: notes(self.default_footer_note, "defaultFooterNote", e),
        }
    }
}

impl DefaultsUpdate {
    /// Set the given fields on the row; the others stay untouched.
    pub fn apply(self, row: &mut company::ActiveModel) {
        if let Some(m) = self.rounding_cash {
            row.default_rounding_cash = Set(m);
        }
        if let Some(m) = self.rounding_other {
            row.default_rounding_other = Set(m);
        }
        if let Some(m) = self.payment_method {
            row.default_payment_method = Set(m);
        }
        if let Some(c) = self.currency {
            row.default_currency = Set(c);
        }
        if let Some(n) = self.header_note {
            row.default_header_note_cs = Set(n.cs);
            row.default_header_note_en = Set(n.en);
        }
        if let Some(n) = self.footer_note {
            row.default_footer_note_cs = Set(n.cs);
            row.default_footer_note_en = Set(n.en);
        }
    }
}

impl TryFrom<&company::Model> for DocDefaults {
    type Error = anyhow::Error;

    fn try_from(m: &company::Model) -> anyhow::Result<Self> {
        Ok(Self {
            default_rounding_cash: m.default_rounding_cash,
            default_rounding_other: m.default_rounding_other,
            default_payment_method: m.default_payment_method,
            default_currency: m.default_currency.clone(),
            default_header_note: LocaleNotes {
                cs: m.default_header_note_cs.clone(),
                en: m.default_header_note_en.clone(),
            },
            default_footer_note: LocaleNotes {
                cs: m.default_footer_note_cs.clone(),
                en: m.default_footer_note_en.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_fields_keep_and_bad_ones_are_reported() {
        let mut e = FieldErrors::new();
        assert_eq!(
            DocDefaultsInput::default().validate(&mut e),
            DefaultsUpdate::default()
        );
        assert!(e.is_empty());

        let bad = DocDefaultsInput {
            default_rounding_cash: Some("whole".into()),
            default_rounding_other: Some("half".into()),
            default_payment_method: Some("cheque".into()),
            default_currency: Some("EURO".into()),
            default_header_note: Some(LocaleNotes {
                cs: Some("x".repeat(NOTE_MAX + 1)),
                en: Some("  ".into()),
            }),
            default_footer_note: Some(LocaleNotes {
                cs: None,
                en: Some("y".repeat(NOTE_MAX + 1)),
            }),
        };
        let u = bad.validate(&mut e);
        assert_eq!(u.rounding_other, Some(RoundingMode::Half));
        let mut expected = FieldErrors::new();
        expected.add("defaultRoundingCash", "invalid");
        expected.add("defaultPaymentMethod", "invalid");
        expected.add("defaultCurrency", "invalid");
        expected.add("defaultHeaderNote.cs", "too_long");
        expected.add("defaultFooterNote.en", "too_long");
        assert_eq!(e, expected);
    }

    #[test]
    fn notes_are_trimmed_and_blank_is_null() {
        let mut e = FieldErrors::new();
        let u = DocDefaultsInput {
            default_header_note: Some(LocaleNotes {
                cs: Some("  Děkujeme  ".into()),
                en: Some("".into()),
            }),
            ..Default::default()
        }
        .validate(&mut e);
        assert!(e.is_empty());
        assert_eq!(
            u.header_note,
            Some(LocaleNotes {
                cs: Some("Děkujeme".into()),
                en: None,
            })
        );
        assert_eq!(u.footer_note, None);
    }
}
