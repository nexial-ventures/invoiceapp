//! E-mail routes: settings (status, test, templates) and per-document
//! prefill / send / history.

pub mod document;
pub mod settings;

use lettre::message::Mailbox;
use minijinja::Value;
use sea_orm::DatabaseConnection;

use super::Mailer;
use super::context::{self, Company};
use super::input::parse_address;
use crate::app::AppState;
use crate::error::AppError;
use crate::pdf::format::Locale;
use crate::pdf::preview::Sample;
use crate::settings::entity::company;

/// The kill switch wins over a missing SMTP configuration.
fn mailer(state: &AppState) -> Result<&Mailer, AppError> {
    if state.email_disabled {
        return Err(AppError::EmailDisabled);
    }
    state.email.as_ref().ok_or(AppError::SmtpNotConfigured)
}

/// The company e-mail as `Reply-To`; an address that does not parse is
/// left out rather than failing the send.
fn reply_to(company: &company::Model) -> Option<Mailbox> {
    let email = company.email.as_deref()?.trim();
    if email.is_empty() {
        return None;
    }
    let parsed = parse_address(email);
    if parsed.is_none() {
        tracing::warn!(email, "company e-mail is not a valid address, no Reply-To");
    }
    parsed
}

/// The contexts templates are validated on: the PDF preview's sample
/// invoice and its variants ([`context::samples`]), named by their prefix.
async fn sample_contexts(
    db: &DatabaseConnection,
    company: &company::Model,
    locale: Locale,
) -> Result<Vec<(&'static str, Value)>, AppError> {
    let sample = Sample::load(db, company, locale).await?;
    Ok(
        context::samples(sample.input(), locale, &Company::from_row(company))
            .into_iter()
            .map(|(name, ctx)| (name, Value::from_serialize(&ctx)))
            .collect(),
    )
}
