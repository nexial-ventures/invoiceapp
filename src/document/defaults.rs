//! Party defaults for new documents: the contact's own setting wins over the
//! company's.

use chrono::{Days, NaiveDate};

use crate::contact::entity::contact;
use crate::document::line::{PaymentMethod, RoundingMode};
use crate::settings::entity::company;

/// `from` + (contact ?? company) default due days.
pub fn due_date(
    contact: Option<&contact::Model>,
    company: &company::Model,
    from: NaiveDate,
) -> NaiveDate {
    let days = contact
        .and_then(|c| c.default_due_days)
        .unwrap_or(company.default_due_days);
    from.checked_add_days(Days::new(u64::try_from(days).unwrap_or(0)))
        .unwrap_or(from)
}

/// (contact ?? company) default document locale.
pub fn locale(contact: Option<&contact::Model>, company: &company::Model) -> String {
    contact
        .and_then(|c| c.default_locale.clone())
        .unwrap_or_else(|| company.default_locale.clone())
}

/// (contact ?? company) default currency.
pub fn currency(contact: Option<&contact::Model>, company: &company::Model) -> String {
    contact
        .and_then(|c| c.default_currency.clone())
        .unwrap_or_else(|| company.default_currency.clone())
}

/// The company's default rounding for `method`: cash vs. every other method.
pub fn rounding_mode(method: PaymentMethod, company: &company::Model) -> RoundingMode {
    if method == PaymentMethod::Cash {
        company.default_rounding_cash
    } else {
        company.default_rounding_other
    }
}

/// The company's default (header, footer) note for `locale`.
pub fn notes(company: &company::Model, locale: &str) -> (Option<String>, Option<String>) {
    match locale {
        "en" => (
            company.default_header_note_en.clone(),
            company.default_footer_note_en.clone(),
        ),
        _ => (
            company.default_header_note_cs.clone(),
            company.default_footer_note_cs.clone(),
        ),
    }
}
