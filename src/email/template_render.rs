//! Rendering e-mail templates: MiniJinja plain text (strict, no
//! auto-escape) under limits, since a template is space-member input run on
//! the server — fuel ([`FUEL`] instructions per part), a recursion limit, an
//! output cap ([`OUTPUT_MAX`] per part) and a wall-clock timeout
//! ([`TIMEOUT`]) around the render on the blocking pool. Exceeding any of
//! them is 422 `template_invalid` with a readable `detail`.

use std::io;
use std::time::Duration;

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
/// Wall-clock limit of one part's render.
pub const TIMEOUT: Duration = Duration::from_secs(5);

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

fn render_now(field: &'static str, source: &str, ctx: &Value) -> Result<String, AppError> {
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

/// [`render_now`] on the blocking pool, given up after `limit` (the thread
/// itself still stops at the fuel limit).
async fn render_part_within(
    limit: Duration,
    field: &'static str,
    source: &str,
    ctx: &Value,
) -> Result<String, AppError> {
    let (source, ctx) = (source.to_string(), ctx.clone());
    let task = tokio::task::spawn_blocking(move || render_now(field, &source, &ctx));
    match tokio::time::timeout(limit, task).await {
        Ok(joined) => joined.context("e-mail template render task")?,
        Err(_) => Err(AppError::TemplateInvalid {
            field,
            detail: format!("rendering took longer than {} s", limit.as_secs()),
        }),
    }
}

/// Save / preview check: render on every sample variant (`(prefix,
/// context)`, see `context::samples`), so strict errors inside branches the
/// base sample does not take are caught at save time. Returns the first
/// variant's rendering; a failure's detail is led by its variant's prefix.
pub async fn validate(t: &Template, samples: &[(&str, Value)]) -> Result<Rendered, AppError> {
    let mut first = None;
    for (prefix, ctx) in samples {
        let rendered = render(t, ctx).await.map_err(|e| match e {
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
pub async fn render(t: &Template, ctx: &Value) -> Result<Rendered, AppError> {
    let subject = render_part_within(TIMEOUT, "subject", &t.subject, ctx).await?;
    let body = render_part_within(TIMEOUT, "body", &t.body, ctx).await?;
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
#[path = "template_render_tests.rs"]
mod tests;
