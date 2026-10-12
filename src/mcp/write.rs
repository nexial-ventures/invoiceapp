//! Write tools: contacts, drafts, issue, payments, mark sent.
//!
//! Once a write has committed, the tool never reports failure: a failing
//! follow-up read is logged and the known id / payment returned
//! ([`written`]), so a client does not retry (and duplicate) the write.

use rmcp::model::{CallToolResult, JsonObject};
use rmcp::{tool, tool_router};
use serde_json::{Value, json};
use uuid::Uuid;

use super::args::{input, parse};
use super::result::{respond, to_value, written};
use super::{InvoiceMcp, document_value, in_space, params};
use crate::app::AppState;
use crate::contact::handlers::dto::{Contact, ContactInput};
use crate::contact::repo::contacts as contact_repo;
use crate::document::entity::document;
use crate::document::line::Status;
use crate::document::repo::{issue, query, view};
use crate::document::service::{self, create_draft, mark_sent_at, update_draft};
use crate::document::state::document_payment_state;
use crate::document::wire::dto::PaymentInput;
use crate::document::wire::input::DocumentInput;
use crate::error::AppError;
use crate::settings::doc_type::{DocType, ISSUED};
use crate::space::{Role, SpaceId};
use crate::time::today;
use axum::http::request::Parts;
use rmcp::handler::server::tool::Extension;

#[tool_router(router = write_tools, vis = "pub(super)")]
impl InvoiceMcp {
    #[tool(
        description = "Create a contact (customer / supplier). ARES is not called: use lookup_ares first and pass its data.",
        input_schema = input::<ContactInput>(),
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_contact(
        &self,
        Extension(parts): Extension<Parts>,
        args: JsonObject,
    ) -> CallToolResult {
        respond(
            in_space(&parts, Role::Member, |space| {
                create_contact(&self.state, space, args)
            })
            .await,
        )
    }

    #[tool(
        description = "Create a draft of an issued invoice, proforma or simplified tax document (docType invoice | proforma | simplified). Omitted fields get defaults: issueDate today, dueDate from the contact / company due days, currency and locale from the contact / company, vatMode from the company, the currency's default bank account, paymentMethod from the company, roundingMode (none | half | unit) from the company default for the payment method (cash / other), headerNote / footerNote from the company notes of the locale. Amounts may be decimal strings or numbers. Check the returned totals, then issue_document.",
        input_schema = input::<DocumentInput>(),
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_draft(
        &self,
        Extension(parts): Extension<Parts>,
        args: JsonObject,
    ) -> CallToolResult {
        respond(
            in_space(&parts, Role::Member, |space| {
                create(&self.state, space, args)
            })
            .await,
        )
    }

    #[tool(
        description = "Replace every field of a draft invoice / proforma / simplified document (like PUT: send the full document, lines included). Only such drafts can be edited (document_locked once issued, invalid_state for other draft types).",
        input_schema = input::<params::UpdateDraft>(),
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_draft(
        &self,
        Extension(parts): Extension<Parts>,
        args: JsonObject,
    ) -> CallToolResult {
        respond(
            in_space(&parts, Role::Member, |space| {
                update(&self.state, space, args)
            })
            .await,
        )
    }

    #[tool(
        description = "Issue a draft invoice / proforma / simplified document: assigns the document number, snapshots the parties, fetches the ČNB rate for a foreign currency, renders and archives the PDF. Irreversible.",
        input_schema = input::<params::Id>(),
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn issue_document(
        &self,
        Extension(parts): Extension<Parts>,
        args: JsonObject,
    ) -> CallToolResult {
        respond(
            in_space(&parts, Role::Member, |space| {
                issue_document(&self.state, space, args)
            })
            .await,
        )
    }

    #[tool(
        description = "Record a payment of an issued document. Returns { payment, paymentState }. On a VAT payer's proforma this also issues the advance tax document (DDPP); for a foreign currency pass exchangeRate when ČNB has no rate.",
        input_schema = input::<params::AddPayment>(),
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_payment(
        &self,
        Extension(parts): Extension<Parts>,
        args: JsonObject,
    ) -> CallToolResult {
        respond(
            in_space(&parts, Role::Member, |space| {
                add_payment(&self.state, space, args)
            })
            .await,
        )
    }

    #[tool(
        description = "Mark an issued document as sent to the customer (sentAt, default now).",
        input_schema = input::<params::MarkSent>(),
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn mark_sent(
        &self,
        Extension(parts): Extension<Parts>,
        args: JsonObject,
    ) -> CallToolResult {
        respond(
            in_space(&parts, Role::Member, |space| {
                mark_sent(&self.state, space, args)
            })
            .await,
        )
    }
}

/// Drafts MCP may edit and issue: native issued-direction invoices, proformas
/// and simplified documents that stand alone (credit / debit notes, DDPP
/// corrections, settlement invoices and imported documents are UI-only).
fn in_scope(direction: &str, doc_type: &str, imported: bool, related: bool) -> bool {
    let simple = matches!(
        DocType::parse_document(doc_type),
        Some(DocType::Invoice | DocType::Proforma | DocType::Simplified)
    );
    direction == ISSUED && simple && !imported && !related
}

/// The document `id`; a draft outside the MCP scope → `invalid_state`.
async fn scoped(state: &AppState, space: SpaceId, id: Uuid) -> Result<document::Model, AppError> {
    let doc = query::find(&state.db, space, id).await?;
    let related = doc.related_document_id.is_some();
    if doc.status == Status::Draft.as_str()
        && !in_scope(&doc.direction, &doc.doc_type, doc.imported, related)
    {
        return Err(AppError::InvalidState);
    }
    Ok(doc)
}

async fn create_contact(
    state: &AppState,
    space: SpaceId,
    args: JsonObject,
) -> Result<Contact, AppError> {
    let input: ContactInput = parse(args)?;
    Ok(contact_repo::create(&state.db, space, input.validate()?)
        .await?
        .into())
}

async fn create(state: &AppState, space: SpaceId, args: JsonObject) -> Result<Value, AppError> {
    let input: DocumentInput = parse(args)?;
    // Imported documents (own number, no rendering) are out of the MCP scope.
    if input.imported == Some(true) {
        return Err(AppError::field("imported", "invalid"));
    }
    let id = create_draft(state, space, input).await?;
    Ok(written(id, document_value(state, space, id).await))
}

async fn update(state: &AppState, space: SpaceId, mut args: JsonObject) -> Result<Value, AppError> {
    // Parsed apart so field paths inside the (flattened) document survive.
    let id_only: JsonObject = args
        .remove("id")
        .map(|v| ("id".into(), v))
        .into_iter()
        .collect();
    let params::Id { id } = parse(id_only)?;
    let input: DocumentInput = parse(args)?;
    let doc = scoped(state, space, id).await?;
    update_draft(state, space, &doc, input).await?;
    Ok(written(id, document_value(state, space, id).await))
}

async fn issue_document(
    state: &AppState,
    space: SpaceId,
    args: JsonObject,
) -> Result<Value, AppError> {
    let params::Id { id } = parse(args)?;
    scoped(state, space, id).await?;
    let pdf = state.pdf.space(space)?;
    issue::issue(&state.db, &state.cnb, &pdf, space, id, today()).await?;
    Ok(written(id, document_value(state, space, id).await))
}

/// The derived `paymentState` from the document row alone.
async fn payment_state(state: &AppState, space: SpaceId, id: Uuid) -> Result<Value, AppError> {
    let doc = query::find(&state.db, space, id).await?;
    let status = view::status(&doc)?;
    to_value(&document_payment_state(
        &doc.doc_type,
        status,
        doc.paid,
        doc.payable,
    ))
}

async fn add_payment(
    state: &AppState,
    space: SpaceId,
    args: JsonObject,
) -> Result<Value, AppError> {
    let p: params::AddPayment = parse(args)?;
    let input = PaymentInput {
        date: Some(p.date),
        amount: p.amount,
        note: p.note,
        exchange_rate: p.exchange_rate,
    };
    let payment = to_value(&service::add_payment(state, space, p.id, input).await?)?;
    Ok(match payment_state(state, space, p.id).await {
        Ok(ps) => json!({ "payment": payment, "paymentState": ps }),
        Err(e) => {
            e.log();
            json!({ "payment": payment, "paymentState": null, "readError": e.body() })
        }
    })
}

async fn mark_sent(state: &AppState, space: SpaceId, args: JsonObject) -> Result<Value, AppError> {
    let p: params::MarkSent = parse(args)?;
    mark_sent_at(state, space, p.id, p.sent_at).await?;
    Ok(written(p.id, document_value(state, space, p.id).await))
}

#[cfg(test)]
mod tests {
    use super::in_scope;

    #[test]
    fn mcp_scope_of_drafts() {
        for t in ["invoice", "proforma", "simplified"] {
            assert!(in_scope("issued", t, false, false), "{t}");
        }
        for t in [
            "credit_note",
            "debit_note",
            "advance_credit_note",
            "advance_tax_doc",
            "x",
        ] {
            assert!(!in_scope("issued", t, false, false), "{t}");
        }
        assert!(!in_scope("issued", "invoice", true, false), "imported");
        assert!(!in_scope("issued", "invoice", false, true), "settlement");
        assert!(!in_scope("received", "invoice", false, false), "received");
    }
}
