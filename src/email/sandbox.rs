//! E-mail templates are member input, and MiniJinja's fuel bounds work, not
//! memory (`'x' * 100000000 ~ …` doubles into gigabytes). So the server never
//! renders one itself: it runs its own binary as `invoice render-template`
//! (hidden), sends the job as JSON on stdin and reads the result from
//! stdout. The child runs with `RLIMIT_DATA` [`MEMORY_LIMIT`] and
//! `RLIMIT_CPU` [`CPU_LIMIT_SECS`] and no core dumps (Linux; elsewhere only
//! the wall clock applies), an empty environment, and is killed after
//! [`TIMEOUT`]; at most [`MAX_STDOUT`] of its output is read. A child
//! that dies or is killed is 422 `template_invalid` for the part it was on;
//! at most [`CONCURRENCY`] run at once (a wait over [`QUEUE_WAIT`] → 503
//! `busy`).

use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use minijinja::Value;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Semaphore;

use super::template_render;
use super::templates::{Rendered, Template};
use crate::error::AppError;

/// The hidden subcommand the child runs.
pub const CHILD_COMMAND: &str = "render-template";
/// Private writable memory of the child (heap; the binary's code and
/// read-only data do not count, so a large debug binary still starts).
pub const MEMORY_LIMIT: u64 = 256 * 1024 * 1024;
pub const CPU_LIMIT_SECS: u64 = 5;
/// Wall clock of one child, then it is killed.
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// Children running at once (instance-wide).
pub const CONCURRENCY: usize = 2;
pub const QUEUE_WAIT: Duration = Duration::from_secs(2);
/// The child's stdout read at most (progress lines + one rendering of at
/// most 2 × 1 MiB, JSON-escaped).
pub const MAX_STDOUT: u64 = 16 * 1024 * 1024;

/// What the server sends the child.
#[derive(Serialize, Deserialize)]
struct Job {
    template: Template,
    /// `(variant prefix, context)`.
    samples: Vec<(String, serde_json::Value)>,
}

/// The child's last stdout line.
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum Outcome {
    Rendered(Rendered),
    Invalid { field: String, detail: String },
    Failed { message: String },
}

/// Progress lines before each part: `> {prefix}\t{field}`.
const PROGRESS: &str = "> ";

#[derive(Clone)]
pub struct TemplateSandbox {
    exe: Arc<PathBuf>,
    gate: Arc<Semaphore>,
    timeout: Duration,
}

impl TemplateSandbox {
    /// Children run `exe` (the `invoice` binary).
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self {
            exe: Arc::new(exe.into()),
            gate: Arc::new(Semaphore::new(CONCURRENCY)),
            timeout: TIMEOUT,
        }
    }

    /// The running binary: on Linux `/proc/self/exe`, which still opens
    /// the running image after the file was replaced by an upgrade.
    pub fn current() -> anyhow::Result<Self> {
        if cfg!(target_os = "linux") {
            return Ok(Self::new("/proc/self/exe"));
        }
        Ok(Self::new(
            std::env::current_exe().context("locate the invoice binary")?,
        ))
    }

    /// A shorter wall clock (tests).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Both parts on one context.
    pub async fn render(&self, t: &Template, ctx: &Value) -> Result<Rendered, AppError> {
        self.validate(t, &[("", ctx.clone())]).await
    }

    /// [`template_render::validate_with`] in a child.
    pub async fn validate(
        &self,
        t: &Template,
        samples: &[(&str, Value)],
    ) -> Result<Rendered, AppError> {
        let _permit = tokio::time::timeout(QUEUE_WAIT, self.gate.acquire())
            .await
            .map_err(|_| AppError::Busy)?
            .context("template sandbox gate closed")?;
        let job = Job {
            template: t.clone(),
            samples: samples
                .iter()
                .map(|(p, v)| Ok((p.to_string(), serde_json::to_value(v)?)))
                .collect::<Result<_, serde_json::Error>>()
                .context("serialize the template context")?,
        };
        let input = serde_json::to_vec(&job).context("serialize the template job")?;
        self.run(&input).await
    }

    async fn run(&self, input: &[u8]) -> Result<Rendered, AppError> {
        let mut cmd = tokio::process::Command::new(&*self.exe);
        cmd.arg(CHILD_COMMAND)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        limit(&mut cmd);
        let mut child = cmd.spawn().context("start the template renderer")?;
        let mut stdin = child.stdin.take().context("renderer stdin")?;
        let stdout = child.stdout.take().context("renderer stdout")?;
        // Where the child was when it stopped: (prefix, field).
        let mut at = (String::new(), "subject");
        let mut last = String::new();
        let mut overflow = false;
        let talk = async {
            stdin.write_all(input).await?;
            drop(stdin);
            let mut lines = BufReader::new(stdout.take(MAX_STDOUT)).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        match line.strip_prefix(PROGRESS).and_then(|l| l.split_once('\t')) {
                            Some((prefix, field)) => at = (prefix.to_string(), part(field)),
                            None => last = line,
                        }
                    }
                    Ok(None) => break,
                    // Cut inside a UTF-8 sequence by the limit.
                    Err(_) => {
                        overflow = true;
                        break;
                    }
                }
            }
            if overflow || lines.into_inner().into_inner().limit() == 0 {
                overflow = true;
                // It may be blocked writing the rest.
                let _ = child.start_kill();
            }
            child.wait().await
        };
        let status = tokio::time::timeout(self.timeout, talk).await;
        let (prefix, field) = at;
        let killed = |detail: String| {
            template_render::with_prefix(&prefix, AppError::TemplateInvalid { field, detail })
        };
        let status = match status {
            Ok(status) => status.context("talk to the template renderer")?,
            Err(_) => {
                // Dropping the future dropped nothing that owns the child.
                let _ = child.kill().await;
                return Err(killed(format!(
                    "rendering took longer than {} s",
                    self.timeout.as_secs()
                )));
            }
        };
        if overflow {
            return Err(killed("too much output".into()));
        }
        if !status.success() {
            tracing::warn!(%status, "e-mail template renderer died");
            return Err(killed("the template needs too much memory or time".into()));
        }
        let Ok(outcome) = serde_json::from_str(&last) else {
            return Err(killed("too much output".into()));
        };
        match outcome {
            Outcome::Rendered(r) => Ok(r),
            Outcome::Invalid { field, detail } => Err(AppError::TemplateInvalid {
                field: part(&field),
                detail,
            }),
            Outcome::Failed { message } => {
                Err(anyhow::anyhow!("template renderer failed: {message}").into())
            }
        }
    }
}

fn part(field: &str) -> &'static str {
    if field == "subject" {
        "subject"
    } else {
        "body"
    }
}

#[cfg(target_os = "linux")]
fn limit(cmd: &mut tokio::process::Command) {
    let rlimit = |value: u64| libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    let (data, cpu, core) = (rlimit(MEMORY_LIMIT), rlimit(CPU_LIMIT_SECS), rlimit(0));
    // SAFETY: the closure runs between fork and exec and only calls
    // setrlimit (async-signal-safe, no allocation), which only reads the
    // limits it is given.
    unsafe {
        cmd.pre_exec(move || {
            // No core dump of a killed bomb.
            if libc::setrlimit(libc::RLIMIT_DATA, &data) != 0
                || libc::setrlimit(libc::RLIMIT_CPU, &cpu) != 0
                || libc::setrlimit(libc::RLIMIT_CORE, &core) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn limit(_: &mut tokio::process::Command) {}

/// `invoice render-template`: one job from stdin, progress and the outcome
/// on stdout. Single-threaded and without a runtime, so the memory limit is
/// not spent on thread stacks and allocator arenas.
pub fn child_main() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("read the job")?;
    let job: Job = serde_json::from_str(&input).context("parse the job")?;
    let samples: Vec<(&str, Value)> = job
        .samples
        .iter()
        .map(|(p, ctx)| (p.as_str(), Value::from_serialize(ctx)))
        .collect();
    let mut out = std::io::stdout().lock();
    let result = template_render::validate_with(&job.template, &samples, &mut |prefix, field| {
        // A lost progress line only makes a later crash name the wrong part.
        let _ = writeln!(out, "{PROGRESS}{prefix}\t{field}");
        let _ = out.flush();
    });
    let outcome = match result {
        Ok(r) => Outcome::Rendered(r),
        Err(AppError::TemplateInvalid { field, detail }) => Outcome::Invalid {
            field: field.to_string(),
            detail,
        },
        Err(e) => Outcome::Failed {
            message: e.to_string(),
        },
    };
    let line = serde_json::to_string(&outcome).context("serialize the outcome")?;
    writeln!(out, "{line}").context("write the outcome")?;
    out.flush().context("flush the outcome")
}
