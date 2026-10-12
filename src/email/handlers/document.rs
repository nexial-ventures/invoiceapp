//! `/api/documents/{id}/email` (prefill, send) and `/api/documents/{id}/emails`.

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, FixedOffset, SubsecRound};
use minijinja::Value;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::{mailer, reply_to};
use crate::app::AppState;
use crate::auth::{Read, Write};
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::document::line::Status;
use crate::document::repo::{lifecycle, query};
use crate::email::context::{self, Company};
use crate::email::input::{SendInput, validate};
use crate::email::limits::{self, Kind};
use crate::email::log::{self, Attempt, EmailLogEntry};
use crate::email::sender::{File, Outgoing};
use crate::email::templates;
use crate::error::{AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::isdoc::export;
use crate::pdf::PdfService;
use crate::pdf::format::Locale;
use crate::pdf::{archive, source};
use crate::settings::repo::company;
use crate::space::SpaceId;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PrefillQuery {
    /// `cs` | `en`; default: the document's locale.
    pub locale: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AttachmentInfo {
    pub available: bool,
    pub filename: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Attachments {
    pub pdf: AttachmentInfo,
    pub isdoc: AttachmentInfo,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Prefill {
    pub configured: bool,
    /// `INVOICE__EMAIL_DISABLED`: sending → 503 `email_disabled`.
    pub disabled: bool,
    pub locale: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub attachments: Attachments,
}

/// Only an issued-direction, non-draft document is sent: received → 404,
/// draft → 409.
fn check_sendable(row: &document::Model) -> Result<(), AppError> {
    if row.direction != Direction::Issued {
        return Err(AppError::NotFound);
    }
    if row.status == Status::Draft {
        return Err(AppError::InvalidState);
    }
    Ok(())
}

/// False only for an imported document without its original.
fn pdf_available(row: &document::Model) -> bool {
    !row.imported || row.original_path.is_some()
}

async fn live_contact_email(
    db: &DatabaseConnection,
    space: SpaceId,
    contact_id: Option<Uuid>,
) -> Result<Option<String>, AppError> {
    let Some(cid) = contact_id else {
        return Ok(None);
    };
    Ok(crate::document::repo::context::contact(db, space, cid)
        .await?
        .and_then(|c| c.email))
}

#[utoipa::path(
    get,
    path = "/api/documents/{id}/email",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path), PrefillQuery),
    responses(
        (status = 200, body = Prefill),
        (status = 404, description = "`not_found` (also a received document)", body = ErrorBody),
        (status = 409, description = "`invalid_state` (a draft)", body = ErrorBody),
        (status = 422, description = "`locale`: `invalid`; `template_invalid`", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`, `busy` (every template renderer taken)", body = ErrorBody),
    )
)]
pub async fn prefill(
    State(state): State<AppState>,
    access: Read,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(q): ApiQuery<PrefillQuery>,
) -> Result<Json<Prefill>, AppError> {
    let space = access.space();
    let pdf = state.pdf.space(space)?;
    let src = source::load(&state.db, space, id).await?;
    check_sendable(&src.row)?;
    let locale = match q.locale.as_deref() {
        Some(l) => Locale::parse(l).ok_or(AppError::field("locale", "invalid"))?,
        None => Locale::parse(&src.row.locale).unwrap_or(Locale::Cs),
    };
    let (company, (template, _), live_email) = tokio::try_join!(
        company::get(&state.db, space),
        templates::load(pdf.storage(), locale),
        live_contact_email(&state.db, space, src.row.contact_id),
    )?;
    let ctx = context::build(
        &src.input()?,
        locale,
        src.doc.paid,
        Company::from_row(&company),
        live_email.as_deref(),
    );
    let rendered = state
        .templates
        .render(&template, &Value::from_serialize(&ctx))
        .await?;
    let to = ctx.contact.and_then(|c| c.email);
    let row = src.row;
    let stem = export::stem(&row);
    Ok(Json(Prefill {
        configured: state.email.is_some(),
        disabled: state.email_disabled,
        locale: locale.as_str().into(),
        to: to.into_iter().collect(),
        cc: vec![],
        bcc: company
            .email
            .filter(|e| !e.trim().is_empty())
            .into_iter()
            .collect(),
        subject: rendered.subject,
        body: rendered.body,
        attachments: Attachments {
            pdf: AttachmentInfo {
                available: pdf_available(&row),
                filename: format!("{stem}.pdf"),
            },
            isdoc: AttachmentInfo {
                available: true,
                filename: format!("{stem}.isdoc"),
            },
        },
    }))
}

/// The files to attach, prepared (concurrently) before anything is sent.
async fn files(
    state: &AppState,
    pdf_service: &PdfService,
    row: &document::Model,
    input: &SendInput,
) -> Result<Vec<File>, AppError> {
    let pdf = async {
        if !input.attach_pdf {
            return Ok(None);
        }
        let (_, body) = archive::document_pdf(&state.db, pdf_service, row.id).await?;
        Ok::<_, AppError>(Some(File {
            filename: format!("{}.pdf", export::stem(row)),
            content_type: "application/pdf",
            bytes: body.into_bytes().await?,
        }))
    };
    let isdoc = async {
        if !input.attach_isdoc {
            return Ok(None);
        }
        let x = export::plain(&state.db, pdf_service.space_id(), row.id).await?;
        Ok::<_, AppError>(Some(File {
            filename: x.filename,
            content_type: x.content_type,
            bytes: x.bytes.into(),
        }))
    };
    let (pdf, isdoc) = tokio::try_join!(pdf, isdoc)?;
    Ok(pdf.into_iter().chain(isdoc).collect())
}

/// The message is out: a failure from here on must not become an error
/// response, or the user would send a delivered e-mail again.
async fn record_sent(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    entry: &EmailLogEntry,
    sent_at: DateTime<FixedOffset>,
) {
    if let Err(e) = log::insert(db, id, entry).await {
        tracing::error!(document = %id, message_id = ?entry.message_id, error = %e, "e-mail sent but not logged");
    }
    if let Err(e) = lifecycle::email_sent(db, space, id, sent_at).await {
        tracing::error!(document = %id, error = %e, "e-mail sent but sentAt not updated");
    }
}

#[utoipa::path(
    post,
    path = "/api/documents/{id}/email",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = SendInput,
    responses(
        (status = 200, description = "Sent and logged; the document's `sentAt` is now", body = EmailLogEntry),
        (status = 404, description = "`not_found` (also a received document)", body = ErrorBody),
        (status = 409, description = "`invalid_state` (a draft)", body = ErrorBody),
        (status = 422, description = "`to` / `cc` / `bcc` (`required`, `to.N`: `invalid`, `too_long` over 10 recipients), `subject` (`required`, `too_long`, `invalid`), `body` (`too_long`), `attachPdf` (`invalid`)", body = ErrorBody),
        (status = 429, description = "`rate_limited` + `Retry-After`: the space's or the user's e-mails per hour / day are used up", body = ErrorBody),
        (status = 502, description = "`smtp_failed` (+ `detail`), logged with `ok: false`; `pdf_render_failed`", body = ErrorBody),
        (status = 503, description = "`email_disabled`, `smtp_not_configured`, `pdf_unavailable`, `storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn send(
    State(state): State<AppState>,
    access: Write,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<SendInput>,
) -> Result<Json<EmailLogEntry>, AppError> {
    let space = access.space();
    let row = query::find(&state.db, space, id).await?;
    check_sendable(&row)?;
    let mailer = mailer(&state)?;
    let v = validate(&input, pdf_available(&row)).map_err(AppError::Validation)?;
    let pdf = state.pdf.space(space)?;
    let (files, company) = tokio::try_join!(
        files(&state, &pdf, &row, &input),
        company::get(&state.db, space)
    )?;
    let attempt = Attempt {
        to: v.to.texts,
        cc: v.cc.texts,
        bcc: v.bcc.texts,
        subject: v.subject.clone(),
        body: v.body.clone(),
        attachments: files.iter().map(|f| f.filename.clone()).collect(),
    };
    let msg = Outgoing {
        reply_to: reply_to(&company),
        to: v.to.mailboxes,
        cc: v.cc.mailboxes,
        bcc: v.bcc.mailboxes,
        subject: v.subject,
        body: v.body,
        files,
    };
    limits::reserve(&state.db, space, access.scope.user_id, Kind::Document).await?;
    let message_id = mailer.message_id();
    let result = mailer.send(msg, &message_id).await;
    // Microseconds, like the stored column, so the entry returned now equals
    // the one the history returns later.
    let now = chrono::Utc::now().trunc_subsecs(6).into();
    match result {
        Ok(()) => {
            let entry = log::entry(attempt, now, None, Some(message_id));
            record_sent(&state.db, space, id, &entry, now).await;
            Ok(Json(entry))
        }
        Err(AppError::SmtpFailed(detail)) => {
            let entry = log::entry(attempt, now, Some(detail.clone()), None);
            if let Err(e) = log::insert(&state.db, id, &entry).await {
                tracing::error!(document = %id, error = %e, "failed e-mail attempt not logged");
            }
            Err(AppError::SmtpFailed(detail))
        }
        Err(e) => Err(e),
    }
}

#[utoipa::path(
    get,
    path = "/api/documents/{id}/emails",
    tag = "email",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Newest first", body = [EmailLogEntry]),
        (status = 404, description = "`not_found` (also a received document)", body = ErrorBody),
    )
)]
pub async fn history(
    State(state): State<AppState>,
    access: Read,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Vec<EmailLogEntry>>, AppError> {
    let row = query::find(&state.db, access.space(), id).await?;
    if row.direction != Direction::Issued {
        return Err(AppError::NotFound);
    }
    Ok(Json(log::list(&state.db, id).await?))
}
