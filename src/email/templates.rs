//! E-mail templates: MiniJinja plain text (strict, no auto-escape), one
//! subject + body per locale. Defaults are embedded; overrides live in the
//! storage as one object per locale, `email/templates/{locale}.json`
//! (`{"subject","body"}`), so a save replaces both parts atomically.

use anyhow::Context as _;
use bytes::Bytes;
use minijinja::{AutoEscape, Environment, UndefinedBehavior, Value};
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
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

fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.set_formatter(|out, state, value| {
        if value.is_none() {
            Ok(())
        } else {
            minijinja::escape_formatter(out, state, value)
        }
    });
    env
}

/// `line N: message` for the client.
fn describe(e: &minijinja::Error) -> String {
    let message = match e.detail() {
        Some(d) => format!("{}: {d}", e.kind()),
        None => e.kind().to_string(),
    };
    match e.line() {
        Some(line) => format!("line {line}: {message}"),
        None => message,
    }
}

fn render_one(
    env: &Environment<'_>,
    field: &'static str,
    source: &str,
    ctx: &Value,
) -> Result<String, AppError> {
    let invalid = |e: minijinja::Error| AppError::TemplateInvalid {
        field,
        detail: describe(&e),
    };
    env.template_from_str(source)
        .map_err(invalid)?
        .render(ctx)
        .map_err(invalid)
}

/// Save / preview check: render on every sample variant (`(prefix,
/// context)`, see `context::samples`), so strict errors inside branches the
/// base sample does not take are caught at save time. Returns the first
/// variant's rendering; a failure's detail is led by its variant's prefix.
pub fn validate(t: &Template, samples: &[(&str, Value)]) -> Result<Rendered, AppError> {
    let mut first = None;
    for (prefix, ctx) in samples {
        let rendered = render(t, ctx).map_err(|e| match e {
            AppError::TemplateInvalid { field, detail } if !prefix.is_empty() => {
                AppError::TemplateInvalid {
                    field,
                    detail: format!("{prefix}: {detail}"),
                }
            }
            other => other,
        })?;
        first.get_or_insert(rendered);
    }
    first
        .context("no sample to validate the template on")
        .map_err(AppError::from)
}

/// Render both parts on `ctx` (subject first): the subject trimmed with its
/// newlines turned into spaces, the body with trailing whitespace trimmed.
pub fn render(t: &Template, ctx: &Value) -> Result<Rendered, AppError> {
    let env = env();
    let subject = render_one(&env, "subject", &t.subject, ctx)?;
    let body = render_one(&env, "body", &t.body, ctx)?;
    Ok(Rendered {
        subject: subject
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        body: body.trim_end().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(paid: bool, to_pay: bool) -> Value {
        Value::from_serialize(json!({
            "doc": {
                "type": "invoice", "typeLabel": "Faktura – daňový doklad", "number": "20260001",
                "issueDate": "1. 10. 2026", "taxDate": "1. 10. 2026", "dueDate": "15. 10. 2026",
                "total": "1 210,00 Kč", "payable": "1 210,00 Kč", "currency": "CZK",
                "paid": paid, "cancelled": false, "toPay": to_pay, "variableSymbol": "20260001",
                "bankAccount": null, "iban": null, "originalNumber": null
            },
            "company": { "name": "Dodavatel s.r.o.", "email": null, "phone": null, "web": null },
            "contact": null
        }))
    }

    #[test]
    fn defaults_render_on_every_state() {
        for locale in [Locale::Cs, Locale::En] {
            for (paid, to_pay) in [(false, true), (true, false), (false, false)] {
                let r = render(&default(locale), &ctx(paid, to_pay)).expect("renders");
                assert!(r.subject.ends_with("20260001 – Dodavatel s.r.o."), "{r:?}");
                assert!(!r.body.ends_with('\n'));
                assert!(r.body.contains("1 210,00 Kč"));
                assert!(!r.body.contains("\n\n\n"), "{}", r.body);
            }
        }
        let cs = render(&default(Locale::Cs), &ctx(false, true)).expect("renders");
        assert!(
            cs.body
                .contains("uhraďte do 15. 10. 2026 pod variabilním symbolem 20260001.")
        );
        let paid = render(&default(Locale::Cs), &ctx(true, false)).expect("renders");
        assert!(paid.body.contains("Neplaťte – již uhrazeno."));
        assert!(!paid.body.contains("uhraďte"));
    }

    #[test]
    fn none_is_empty_and_no_html_escaping() {
        let t = Template {
            subject: "A{{ doc.bankAccount }}B\n  {{ company.name }} <&>".into(),
            body: "{{ contact }}|{{ '<b>' }}\n\n  ".into(),
        };
        let r = render(&t, &ctx(false, true)).expect("renders");
        assert_eq!(r.subject, "AB Dodavatel s.r.o. <&>");
        assert_eq!(r.body, "|<b>");
    }

    #[test]
    fn errors_name_the_part_and_line() {
        let ctx = ctx(false, true);
        let undefined = Template {
            subject: "ok".into(),
            body: "a\nb\n{{ doc.nope }}".into(),
        };
        match render(&undefined, &ctx) {
            Err(AppError::TemplateInvalid { field, detail }) => {
                assert_eq!(field, "body");
                assert!(detail.starts_with("line 3: undefined value"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
        let syntax = Template {
            subject: "{% if %}".into(),
            body: "{{ doc.nope }}".into(),
        };
        assert!(matches!(
            render(&syntax, &ctx),
            Err(AppError::TemplateInvalid {
                field: "subject",
                ..
            })
        ));
        // Attribute of none is undefined too (strict).
        let contact = Template {
            subject: "{{ contact.email }}".into(),
            body: String::new(),
        };
        assert!(render(&contact, &ctx).is_err());
    }

    #[test]
    fn validation_renders_every_sample() {
        let mut base = json!({
            "doc": { "number": "1", "paid": false },
            "contact": { "name": "A", "email": "a@x.cz" }
        });
        let mut samples = vec![("", Value::from_serialize(&base))];
        base["doc"]["paid"] = json!(true);
        samples.push(("paid", Value::from_serialize(&base)));
        base["doc"]["paid"] = json!(false);
        base["contact"] = serde_json::Value::Null;
        samples.push(("without contact", Value::from_serialize(&base)));

        let unguarded = Template {
            subject: "{{ doc.number }}".into(),
            body: "x\n{{ contact.name }}".into(),
        };
        match validate(&unguarded, &samples) {
            Err(AppError::TemplateInvalid { field, detail }) => {
                assert_eq!(field, "body");
                assert!(detail.starts_with("without contact: line 2: "), "{detail}");
            }
            other => panic!("{other:?}"),
        }
        let untaken = Template {
            subject: "{% if doc.paid %}{{ doc.nope }}{% endif %}x".into(),
            body: String::new(),
        };
        assert!(render(&untaken, &samples[0].1).is_ok());
        assert!(matches!(
            validate(&untaken, &samples),
            Err(AppError::TemplateInvalid { field: "subject", detail })
                if detail.starts_with("paid: line 1: undefined value")
        ));
        let guarded = Template {
            subject: "{{ doc.number }}".into(),
            body: "{% if contact %}{{ contact.name }}{% endif %}".into(),
        };
        let r = validate(&guarded, &samples).expect("valid");
        assert_eq!(r.body, "A", "the result is the base sample's rendering");
        // An error on the base sample keeps its plain detail.
        let broken = Template {
            subject: "{{ doc.nope }}".into(),
            body: String::new(),
        };
        assert!(matches!(
            validate(&broken, &samples),
            Err(AppError::TemplateInvalid { field: "subject", detail }) if detail.starts_with("line 1:")
        ));
    }

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
