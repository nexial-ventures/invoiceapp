//! Loads what a document's payload is built from: the document itself,
//! the parties (snapshots once issued, live rows for a draft) and the
//! exchange-rate note.

use anyhow::Context as _;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

use super::format::Locale;
use super::payload::{Input, RateInfo};
use crate::document::entity::document;
use crate::document::line::{Status, VatMode};
use crate::document::repo::issue::{customer, supplier};
use crate::document::repo::{context, query, view};
use crate::document::snapshot::{BankSnapshot, PartySnapshot};
use crate::document::wire::dto::Document;
use crate::error::AppError;
use crate::settings::repo::company;
use crate::space::SpaceId;
use crate::time::today;

pub struct Source {
    pub row: document::Model,
    pub doc: Document,
    pub supplier: Option<PartySnapshot>,
    pub customer: Option<PartySnapshot>,
    pub bank: Option<BankSnapshot>,
    pub rate: Option<RateInfo>,
}

/// The printed rate. A credit note copies its invoice's rate (`original`),
/// so it is labelled like the invoice's.
async fn rate_info<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    row: &document::Model,
) -> Result<Option<RateInfo>, AppError> {
    let Some(rate) = row.exchange_rate else {
        return Ok(None);
    };
    let (source, date) = match (row.exchange_rate_source.as_deref(), row.related_document_id) {
        (Some("original"), Some(parent)) => {
            let p = query::find(db, space, parent).await?;
            (p.exchange_rate_source, p.exchange_rate_date)
        }
        _ => (row.exchange_rate_source.clone(), row.exchange_rate_date),
    };
    Ok(Some(RateInfo {
        rate,
        cnb_date: date.filter(|_| source.as_deref() == Some("cnb")),
    }))
}

pub async fn load<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    id: Uuid,
) -> Result<Source, AppError> {
    let full = query::load(db, space, id).await?;
    let row = full.doc.clone();
    let doc = view::document(full, today())?;
    let rate = rate_info(db, space, &row).await?;
    if doc.status != Status::Draft {
        return Ok(Source {
            supplier: doc.supplier.clone(),
            customer: doc.customer.clone(),
            bank: doc.bank_snapshot.clone(),
            row,
            doc,
            rate,
        });
    }
    let company = company::get(db, space).await?;
    let contact = match row.contact_id {
        Some(cid) => context::contact(db, space, cid).await?,
        None => None,
    };
    let bank = match row.bank_account_id {
        Some(bid) => context::bank_account(db, space, bid).await?,
        None => None,
    };
    Ok(Source {
        supplier: Some(supplier(&company)),
        customer: contact.as_ref().map(customer),
        bank: bank.map(|b| BankSnapshot {
            account_number: b.account_number,
            iban: b.iban,
            bic: b.bic,
        }),
        row,
        doc,
        rate,
    })
}

impl Source {
    pub fn input(&self) -> Result<Input<'_>, AppError> {
        let d = &self.doc;
        let locale = Locale::parse(&d.locale)
            .with_context(|| format!("document {} has unknown locale", d.id))?;
        let vat_mode = VatMode::parse(&d.vat_mode)
            .with_context(|| format!("document {} has unknown vat mode", d.id))?;
        Ok(Input {
            locale,
            status: d.status,
            doc_type: &d.doc_type,
            vat_mode,
            number: d.number.as_deref(),
            issue_date: d.issue_date,
            tax_point_date: d.tax_point_date,
            due_date: d
                .due_date
                .with_context(|| format!("document {} has no due date", d.id))?,
            currency: &d.currency,
            rate: self.rate,
            payment_method: &d.payment_method,
            variable_symbol: d.variable_symbol.as_deref(),
            constant_symbol: d.constant_symbol.as_deref(),
            order_ref: d.order_ref.as_deref(),
            header_note: d.header_note.as_deref(),
            footer_note: d.footer_note.as_deref(),
            correction_reason: d.correction_reason.as_deref(),
            parent_number: d.parent.as_ref().and_then(|p| p.number.as_deref()),
            parent_doc_type: d.parent.as_ref().map(|p| p.doc_type.as_str()),
            supplier: self.supplier.as_ref(),
            customer: self.customer.as_ref(),
            bank: self.bank.as_ref(),
            lines: &d.lines,
            totals: &d.totals,
        })
    }
}
