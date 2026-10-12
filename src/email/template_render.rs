//! The e-mail template engine: MiniJinja plain text (strict, no
//! auto-escape) with fuel ([`FUEL`] instructions per part), a recursion
//! limit and an output cap ([`OUTPUT_MAX`] per part); exceeding one is 422
//! `template_invalid` with a readable `detail`. These bound the work, not the
//! memory (`'x' * n ~ …` can still allocate a lot), so the server only runs
//! it inside the child process of [`super::sandbox`].

use std::io;

use anyhow::Context as _;
use minijinja::{AutoEscape, Environment, ErrorKind, UndefinedBehavior, Value};

use super::templates::{Rendered, Template};
use crate::error::AppError;

/// Instructions one part may execute (the defaults use a few hundred).
pub const FUEL: u64 = 100_000;
/// Nesting depth (loops, conditions, macro calls) of one render.
pub const RECURSION_LIMIT: usize = 100;
/// Output bytes of one part.
pub const OUTPUT_MAX: usize = 1024 * 1024;

fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.set_fuel(Some(FUEL));
    env.set_recursion_limit(RECURSION_LIMIT);
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
    let message = match (e.kind(), e.detail()) {
        (ErrorKind::OutOfFuel, _) => format!("too much work (more than {FUEL} steps)"),
        (kind, Some(d)) => format!("{kind}: {d}"),
        (kind, None) => kind.to_string(),
    };
    match e.line() {
        Some(line) => format!("line {line}: {message}"),
        None => message,
    }
}

/// Collects the output, refusing to grow past [`OUTPUT_MAX`].
#[derive(Default)]
struct Capped {
    out: Vec<u8>,
    over: bool,
}

impl io::Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.out.len() + buf.len() > OUTPUT_MAX {
            self.over = true;
            return Err(io::Error::other("output limit"));
        }
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn render_part(field: &'static str, source: &str, ctx: &Value) -> Result<String, AppError> {
    let invalid = |e: minijinja::Error| AppError::TemplateInvalid {
        field,
        detail: describe(&e),
    };
    let env = env();
    let template = env.template_from_str(source).map_err(invalid)?;
    let mut out = Capped::default();
    match template.render_captured_to(ctx, &mut out) {
        Ok(_) => Ok(String::from_utf8(out.out).context("e-mail template output is not UTF-8")?),
        Err(_) if out.over => Err(AppError::TemplateInvalid {
            field,
            detail: format!("the output is larger than {} KB", OUTPUT_MAX / 1024),
        }),
        Err(e) => Err(invalid(e)),
    }
}

/// Render on every sample variant (`(prefix, context)`, see
/// `context::samples`), so strict errors inside branches the base sample
/// does not take are caught at save time. Returns the first variant's
/// rendering; a failure's detail is led by its variant's prefix.
/// `progress(prefix, field)` is called before each part (the sandbox
/// reports it, so a killed render still names its part).
pub fn validate_with(
    t: &Template,
    samples: &[(&str, Value)],
    progress: &mut dyn FnMut(&str, &'static str),
) -> Result<Rendered, AppError> {
    let mut first = None;
    for (prefix, ctx) in samples {
        let rendered = render_with(t, ctx, &mut |field| progress(prefix, field))
            .map_err(|e| with_prefix(prefix, e))?;
        first.get_or_insert(rendered);
    }
    first
        .context("no sample to validate the template on")
        .map_err(AppError::from)
}

/// [`validate_with`] without progress reports (tests).
pub fn validate(t: &Template, samples: &[(&str, Value)]) -> Result<Rendered, AppError> {
    validate_with(t, samples, &mut |_, _| {})
}

/// Lead a `template_invalid` detail with the sample variant's prefix.
pub fn with_prefix(prefix: &str, e: AppError) -> AppError {
    match e {
        AppError::TemplateInvalid { field, detail } if !prefix.is_empty() => {
            AppError::TemplateInvalid {
                field,
                detail: format!("{prefix}: {detail}"),
            }
        }
        other => other,
    }
}

/// Render both parts on `ctx` (subject first): the subject trimmed with its
/// newlines turned into spaces, the body with trailing whitespace trimmed.
fn render_with(
    t: &Template,
    ctx: &Value,
    progress: &mut dyn FnMut(&'static str),
) -> Result<Rendered, AppError> {
    progress("subject");
    let subject = render_part("subject", &t.subject, ctx)?;
    progress("body");
    let body = render_part("body", &t.body, ctx)?;
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

/// Both parts on one context (tests).
pub fn render(t: &Template, ctx: &Value) -> Result<Rendered, AppError> {
    render_with(t, ctx, &mut |_| {})
}

#[cfg(test)]
#[path = "template_render_tests.rs"]
mod tests;
