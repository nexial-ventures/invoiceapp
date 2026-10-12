//! E-mail templates: MiniJinja plain text (strict, no auto-escape), one
//! subject + body per locale. Defaults are embedded; overrides live in the
//! storage as one object per locale, `email/templates/{locale}.json`
//! (`{"subject","body"}`), so a save replaces both parts atomically.
//! Rendering: [`super::template_render`] (the engine and its limits), run in
//! a child process by [`super::sandbox`].

use anyhow::Context as _;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{AppError, FieldErrors};
use crate::pdf::format::Locale;
use crate::storage::{self, Storage};
use crate::validation as v;

pub const SUBJECT_MAX: usize = 500;
pub const BODY_MAX: usize = 20_000;

const PREFIX: &str = "email/templates";

/// The template pair of one locale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Template {
    pub subject: String,
    pub body: String,
}

/// A template being saved or previewed. Both parts are required; they are
/// optional here only so that a missing one is a 422 `required` instead of
/// a serde 400.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct TemplateInput {
    #[schema(required = true)]
    pub subject: Option<String>,
    #[schema(required = true)]
    pub body: Option<String>,
}

/// A rendered message text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Rendered {
    pub subject: String,
    pub body: String,
}

pub fn default(locale: Locale) -> Template {
    let (subject, body) = match locale {
        Locale::Cs => (
            include_str!("defaults/cs/subject.txt"),
            include_str!("defaults/cs/body.txt"),
        ),
        Locale::En => (
            include_str!("defaults/en/subject.txt"),
            include_str!("defaults/en/body.txt"),
        ),
    };
    Template {
        subject: subject.into(),
        body: body.into(),
    }
}

fn key(locale: Locale) -> String {
    format!("{PREFIX}/{}.json", locale.as_str())
}

/// The effective template and whether an override exists.
pub async fn load(storage: &Storage, locale: Locale) -> Result<(Template, bool), AppError> {
    let key = key(locale);
    match storage.get(&key).await {
        Ok(bytes) => {
            let t: Template = serde_json::from_slice(&bytes)
                .with_context(|| format!("stored e-mail template {key} is not valid JSON"))?;
            Ok((t, true))
        }
        Err(storage::Error::NotFound(_)) => Ok((default(locale), false)),
        Err(e) => Err(e.into()),
    }
}

pub async fn save(storage: &Storage, locale: Locale, t: &Template) -> Result<(), AppError> {
    let json = serde_json::to_vec(t).context("serialize e-mail template")?;
    storage.put(&key(locale), Bytes::from(json)).await?;
    Ok(())
}

/// Idempotent.
pub async fn remove(storage: &Storage, locale: Locale) -> Result<(), AppError> {
    storage.delete(&key(locale)).await?;
    Ok(())
}

impl TemplateInput {
    /// Presence and size limits; the parts are kept as given.
    pub fn validate(self) -> Result<Template, AppError> {
        let mut errors = FieldErrors::new();
        let subject = self.subject.unwrap_or_default();
        errors.check("subject", v::required_text(&subject, SUBJECT_MAX));
        match &self.body {
            None => errors.add("body", "required"),
            Some(b) if b.chars().count() > BODY_MAX => errors.add("body", "too_long"),
            Some(_) => {}
        }
        errors.into_result()?;
        Ok(Template {
            subject,
            body: self.body.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits() {
        let ok = default(Locale::Cs);
        let input = TemplateInput {
            subject: Some(ok.subject.clone()),
            body: Some(ok.body.clone()),
        };
        assert_eq!(input.validate().ok(), Some(ok));
        let bad = TemplateInput {
            subject: Some("  ".into()),
            body: Some("x".repeat(BODY_MAX + 1)),
        };
        match bad.validate() {
            Err(AppError::Validation(f)) => {
                assert_eq!(f.get("subject"), Some("required"));
                assert_eq!(f.get("body"), Some("too_long"));
            }
            other => panic!("{other:?}"),
        }
        let long = TemplateInput {
            subject: Some("x".repeat(SUBJECT_MAX + 1)),
            body: Some(String::new()),
        };
        assert!(
            matches!(long.validate(), Err(AppError::Validation(f)) if f.get("subject") == Some("too_long"))
        );
        match TemplateInput::default().validate() {
            Err(AppError::Validation(f)) => {
                assert_eq!(f.get("subject"), Some("required"));
                assert_eq!(f.get("body"), Some("required"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn storage_keys() {
        assert_eq!(key(Locale::En), "email/templates/en.json");
    }
}
