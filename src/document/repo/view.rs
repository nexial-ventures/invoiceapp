//! Stored rows → response DTOs (derived state computed here, at read time).

use std::collections::HashMap;

use anyhow::Context as _;
use chrono::NaiveDate;
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::query::Full;
use crate::auth::entity::user;
use crate::contact::entity::contact;
use crate::currency::is_local;
use crate::document::compute::{self, Params, RecapRow, round2};
use crate::document::doc_type::DocType;
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::document::line::Status;
use crate::document::snapshot::{BankSnapshot, PartySnapshot, Totals};
use crate::document::state;
use crate::document::wire::dto::{self, Document, DocumentSummary, RelatedDocument, lines_out};
use crate::error::AppError;
use crate::pdf::editor::UserRef;

fn snapshot<T: DeserializeOwned>(v: &Option<serde_json::Value>) -> Result<Option<T>, AppError> {
    Ok(v.clone()
        .map(serde_json::from_value)
        .transpose()
        .context("decode document snapshot")?)
}

fn derived(doc: &document::Model, today: NaiveDate) -> (Status, bool) {
    let s = doc.status;
    (
        s,
        doc.due_date.is_some_and(|due| {
            state::is_overdue(doc.doc_type, s, doc.paid, doc.payable, due, today)
        }),
    )
}

/// The stored totals (never recomputed on read).
fn stored_totals(full: &Full) -> compute::Totals {
    let d = &full.doc;
    compute::Totals {
        recap: full
            .recap
            .iter()
            .map(|r| RecapRow {
                vat_rate: r.vat_rate,
                base: round2(r.base),
                vat: round2(r.vat),
                base_czk: r.base_czk.map(round2),
                vat_czk: r.vat_czk.map(round2),
            })
            .collect(),
        base: round2(d.total_base),
        vat: round2(d.total_vat),
        total: round2(d.total),
        rounding: round2(d.rounding),
        payable: round2(d.payable),
        total_czk: d.total_czk.map(round2),
    }
}

fn related(r: &document::Model) -> RelatedDocument {
    RelatedDocument {
        id: r.id,
        doc_type: r.doc_type,
        number: r.number.clone(),
        status: r.status,
        payable: round2(r.payable),
        currency: r.currency.clone(),
    }
}

/// The top-level `total`: received documents show the contract's
/// Σ(base + vat) + rounding, issued ones `totals.total`.
fn top_total(received: bool, t: &Totals) -> Result<rust_decimal::Decimal, AppError> {
    if !received {
        return Ok(t.total);
    }
    Ok(t.total
        .checked_add(t.rounding)
        .map(round2)
        .context("stored received total overflows")?)
}

pub fn document(full: Full, today: NaiveDate) -> Result<Document, AppError> {
    let (status, overdue) = derived(&full.doc, today);
    let d = &full.doc;
    let rounding = d.rounding_mode;
    let params = Params {
        vat_mode: d.vat_mode,
        is_czk: is_local(&d.currency),
        exchange_rate: d.exchange_rate,
        rounding,
    };
    // Stored lines passed validation on save, so this only derives line bases.
    let evaluated = compute::evaluate(&full.lines, params)
        .map_err(|e| anyhow::anyhow!("stored document {} fails validation: {e:?}", d.id))?;
    let totals: Totals = stored_totals(&full).into();
    let received = full.doc.direction == Direction::Received;
    let vat_recap = received.then(|| {
        totals
            .recap
            .iter()
            .map(|r| dto::EnteredRecap {
                rate: r.vat_rate,
                base: r.base,
                vat: r.vat,
            })
            .collect()
    });
    let related_documents: Vec<_> = full.related.iter().map(related).collect();
    let parent = full.parent.as_ref().map(related);
    let unlocked_by = full.unlocked_by;
    let d = full.doc;
    let settled = (d.doc_type == DocType::Proforma).then(|| {
        related_documents
            .iter()
            .any(|r| r.doc_type == DocType::Invoice && r.status != Status::Cancelled)
    });
    Ok(Document {
        payment_state: state::document_payment_state(d.doc_type, status, d.paid, d.payable),
        sign: d.doc_type.sign(),
        pdf: match (d.pdf_sha256.clone(), d.pdf_rendered_at) {
            (Some(sha256), Some(rendered_at)) => Some(dto::PdfArchive {
                sha256,
                rendered_at,
            }),
            _ => None,
        },
        settled,
        correction_block: full.correction_block.map(str::to_string),
        original: match (
            d.original_sha256.clone(),
            d.original_size,
            d.original_uploaded_at,
        ) {
            (Some(sha256), Some(size), Some(uploaded_at)) => Some(dto::OriginalPdf {
                sha256,
                size,
                uploaded_at,
            }),
            _ => None,
        },
        vat_recap,
        rounding: totals.rounding,
        total: top_total(received, &totals)?,
        payable: totals.payable,
        imported: d.imported,
        supplier_number: d.supplier_number,
        received_date: d.received_date,
        vat_deductible: d.vat_deductible,
        supplier_account: d.supplier_account,
        category_id: d.category_id,
        custom_fields: d.custom_fields.as_object().cloned().unwrap_or_default(),
        related_documents,
        parent,
        related_document_id: d.related_document_id,
        payment_id: d.payment_id,
        correction_reason: d.correction_reason,
        overdue,
        status,
        supplier: snapshot(&d.supplier_snapshot)?,
        customer: snapshot(&d.customer_snapshot)?,
        bank_snapshot: snapshot::<BankSnapshot>(&d.bank_snapshot)?,
        lines: lines_out(&full.lines, &evaluated.lines),
        totals,
        id: d.id,
        doc_type: d.doc_type,
        direction: d.direction,
        number: d.number,
        contact_id: d.contact_id,
        issue_date: d.issue_date,
        tax_point_date: d.tax_point_date,
        due_date: d.due_date,
        currency: d.currency,
        exchange_rate: d.exchange_rate.map(|r| r.normalize()),
        exchange_rate_date: d.exchange_rate_date,
        exchange_rate_source: d.exchange_rate_source,
        locale: d.locale,
        vat_mode: d.vat_mode,
        bank_account_id: d.bank_account_id,
        payment_method: d.payment_method,
        variable_symbol: d.variable_symbol,
        constant_symbol: d.constant_symbol,
        order_ref: d.order_ref,
        header_note: d.header_note,
        footer_note: d.footer_note,
        internal_note: d.internal_note,
        rounding_mode: rounding,
        sent_at: d.sent_at,
        cancelled_at: d.cancelled_at,
        cancel_reason: d.cancel_reason,
        unlocked_at: d.unlocked_at,
        unlocked_by,
        unlock_reason: d.unlock_reason,
        paid: round2(d.paid),
        created_at: d.created_at,
        updated_at: d.updated_at,
    })
}

/// The other party's snapshot: the supplier of a received document, else
/// the customer.
fn counterparty(d: &document::Model) -> &Option<serde_json::Value> {
    if d.direction == Direction::Received {
        &d.supplier_snapshot
    } else {
        &d.customer_snapshot
    }
}

/// Summaries; drafts (no snapshot yet) show the live contact name.
pub async fn summaries(
    db: &DatabaseConnection,
    docs: Vec<document::Model>,
    today: NaiveDate,
) -> Result<Vec<DocumentSummary>, AppError> {
    let ids: Vec<Uuid> = docs
        .iter()
        .filter(|d| counterparty(d).is_none())
        .filter_map(|d| d.contact_id)
        .collect();
    let names: HashMap<Uuid, String> = if ids.is_empty() {
        HashMap::new()
    } else {
        contact::Entity::find()
            .filter(contact::Column::Id.is_in(ids))
            .all(db)
            .await?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect()
    };
    let users = user_refs(db, docs.iter().filter_map(|d| d.unlocked_by)).await?;
    docs.into_iter()
        .map(|d| {
            let (status, overdue) = derived(&d, today);
            let snap: Option<PartySnapshot> = snapshot(counterparty(&d))?;
            let customer_name = match snap {
                Some(s) => Some(s.name),
                None => d.contact_id.and_then(|id| names.get(&id).cloned()),
            };
            Ok(DocumentSummary {
                payment_state: state::document_payment_state(d.doc_type, status, d.paid, d.payable),
                sign: d.doc_type.sign(),
                related_document_id: d.related_document_id,
                overdue,
                status,
                customer_name,
                id: d.id,
                doc_type: d.doc_type,
                direction: d.direction,
                number: d.number,
                contact_id: d.contact_id,
                issue_date: d.issue_date,
                due_date: d.due_date,
                currency: d.currency,
                payable: round2(d.payable),
                paid: round2(d.paid),
                sent_at: d.sent_at,
                imported: d.imported,
                has_pdf: d.pdf_path.is_some() || d.original_path.is_some(),
                supplier_number: d.supplier_number,
                category_id: d.category_id,
                unlocked_at: d.unlocked_at,
                unlocked_by: d.unlocked_by.and_then(|u| users.get(&u).cloned()),
                unlock_reason: d.unlock_reason,
            })
        })
        .collect()
}

/// The users of `ids` as "who" references (deleted accounts are missing).
pub async fn user_refs<C: ConnectionTrait>(
    db: &C,
    ids: impl IntoIterator<Item = Uuid>,
) -> Result<HashMap<Uuid, UserRef>, AppError> {
    let ids: Vec<Uuid> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(user::Entity::find()
        .filter(user::Column::Id.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .map(|u| {
            let who = UserRef {
                email: u.email,
                display_name: u.display_name,
            };
            (u.id, who)
        })
        .collect())
}
