//! `draft → issued`: validation, exchange rate, numbering, snapshots, totals.

use bytes::Bytes;
use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;
use sea_orm::{DatabaseConnection, TransactionTrait};
use uuid::Uuid;

use super::{advance_sources, context, issue_apply, meta, original, query};
use crate::cnb::{self, CnbClient};
use crate::contact::entity::contact;
use crate::currency::is_local;
use crate::document::advance::{self, AdvanceCtx};
use crate::document::compute::{Evaluated, Params};
use crate::document::custom_fields;
use crate::document::doc_type::DocType;
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::document::line::{LineData, PaymentMethod, Status};
use crate::document::model;
use crate::document::snapshot::PartySnapshot;
use crate::error::{AppError, FieldErrors};
use crate::pdf::{PdfService, archive};
use crate::settings::entity::{bank_account, company};
use crate::settings::repo::company as company_repo;
use crate::space::SpaceId;

/// The exchange rate an issued document uses.
pub struct Rate {
    pub rate: Option<Decimal>,
    pub date: Option<NaiveDate>,
    pub source: Option<&'static str>,
}

/// CZK: no rate. Otherwise the manual rate, else ČNB for `date`; ČNB failing
/// or not listing the currency means the user has to enter the rate
/// (`exchangeRate: required`).
pub async fn fetch_rate(
    db: &DatabaseConnection,
    cnb: &CnbClient,
    currency: &str,
    manual: Option<Decimal>,
    date: NaiveDate,
    today: NaiveDate,
) -> Result<Rate, AppError> {
    if is_local(currency) {
        return Ok(Rate {
            rate: None,
            date: None,
            source: None,
        });
    }
    if let Some(r) = manual {
        return Ok(Rate {
            rate: Some(r),
            date: None,
            source: Some("manual"),
        });
    }
    match cnb::repo::rate(db, cnb, currency, date, today).await {
        Ok(r) => Ok(Rate {
            rate: Some(r.rate),
            date: Some(r.date),
            source: Some("cnb"),
        }),
        Err(AppError::NotFound | AppError::CnbUnavailable(_)) => {
            Err(AppError::field("exchangeRate", "required"))
        }
        Err(other) => Err(other),
    }
}

/// Everything resolved outside any transaction (incl. the ČNB call, which
/// must not run while the row is locked).
pub(super) struct Prepared {
    pub(super) doc_type: DocType,
    pub(super) imported: bool,
    pub(super) seen_updated_at: DateTime<FixedOffset>,
    pub(super) lines: Vec<LineData>,
    /// `None` only for a simplified document without a contact.
    pub(super) customer: Option<contact::Model>,
    pub(super) bank: Option<bank_account::Model>,
    pub(super) company: company::Model,
    pub(super) rate: Rate,
    pub(super) params: Params,
    pub(super) evaluated: Evaluated,
}

/// Issue attempts when the draft is edited while rendering: the first plus
/// one re-render, then 409 `conflict`.
const EDIT_ATTEMPTS: usize = 2;
/// Re-renders when only the number moved (another issue of the series
/// committed first) — cheap to lose, so a busier budget.
const NUMBER_RETRIES: usize = 5;

/// Issue the draft with its archived PDF, without holding a transaction,
/// row lock or pooled connection across the mdcast render or the storage
/// write:
/// 1. a dry run (transaction rolled back) applies the issue and builds the
///    PDF payload exactly as it will be committed — number included;
/// 2. the payload is rendered and the PDF written under a new per-attempt
///    key, outside any transaction;
/// 3. a short transaction applies the issue again (row lock, `updated_at`
///    check, number allocation) and commits only when it yields the same
///    payload, recording the archive with it.
///
/// A render or storage failure → nothing issued, no number used. A changed
/// document or number → the object is removed and the issue starts over.
pub async fn issue(
    db: &DatabaseConnection,
    cnb: &CnbClient,
    pdf: &PdfService,
    space: SpaceId,
    id: Uuid,
    today: NaiveDate,
) -> Result<(), AppError> {
    let (mut edits, mut races) = (0, 0);
    while edits < EDIT_ATTEMPTS && races <= NUMBER_RETRIES {
        let p = prepare(db, cnb, space, id, today).await?;
        // Imported documents never get our rendered PDF.
        if p.imported {
            let txn = db.begin().await?;
            let result = issue_apply::apply(&txn, space, id, &p).await;
            return issue_apply::finish(txn, result.map(|()| true))
                .await
                .map(|_| ());
        }
        let planned = issue_apply::plan(db, space, id, &p).await?;
        let bytes = Bytes::from(pdf.render(planned.payload).await?);
        let rel = archive::put(pdf, planned.year, id, &bytes).await?;
        let txn = match db.begin().await {
            Ok(txn) => txn,
            Err(e) => {
                pdf.storage().remove(&rel).await;
                return Err(e.into());
            }
        };
        let result =
            issue_apply::commit_planned(&txn, space, id, &p, &planned.json, &rel, &bytes).await;
        match issue_apply::finish(txn, result).await {
            Ok(true) => return Ok(()),
            // Same draft, different payload: another issue took the number.
            Ok(false) => {
                original::remove_unreferenced(db, pdf, &rel).await;
                races += 1;
                tokio::time::sleep(backoff()).await;
            }
            // The draft was edited while rendering: start over with it.
            Err(AppError::Conflict(_)) => {
                original::remove_unreferenced(db, pdf, &rel).await;
                edits += 1;
            }
            Err(e) => {
                original::remove_unreferenced(db, pdf, &rel).await;
                return Err(e);
            }
        }
    }
    Err(AppError::Conflict("document changed while issuing".into()))
}

/// 20–120 ms, so issues that lost the same number race do not collide again.
fn backoff() -> std::time::Duration {
    let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]) % 100;
    std::time::Duration::from_millis(20 + jitter)
}

/// Validate the draft and resolve what the issue needs (no transaction).
async fn prepare(
    db: &DatabaseConnection,
    cnb: &CnbClient,
    space: SpaceId,
    id: Uuid,
    today: NaiveDate,
) -> Result<Prepared, AppError> {
    let doc = query::find(db, space, id).await?;
    if doc.status != Status::Draft {
        return Err(AppError::InvalidState);
    }
    let doc_type = context::existing(&doc)?.doc_type;
    let mut lines = query::load_lines(db, id).await?;
    let (customer, bank) = check(db, space, &doc, doc_type, &lines).await?;
    // A native correction keeps the rate of the document it corrects.
    let rate = if doc_type.is_correction() && !doc.imported {
        Rate {
            rate: doc.exchange_rate,
            date: None,
            source: doc.exchange_rate.map(|_| "original"),
        }
    } else {
        let date = doc.tax_point_date.unwrap_or(doc.issue_date);
        fetch_rate(db, cnb, &doc.currency, doc.exchange_rate, date, today).await?
    };
    let vat_mode = doc.vat_mode;
    let params = Params {
        vat_mode,
        is_czk: is_local(&doc.currency),
        exchange_rate: rate.rate,
        rounding: doc.rounding_mode,
    };
    // Re-resolve deductions: the stored amounts are a draft-time snapshot.
    let sources = advance_sources::load(db, space, &advance::referenced_ids(&lines)).await?;
    let adv = AdvanceCtx {
        doc_type,
        vat_mode,
        document_id: Some(id),
        related_document_id: doc.related_document_id,
        contact_id: doc.contact_id,
        currency: &doc.currency,
        locale: &doc.locale,
        sources: &sources,
    };
    let evaluated = model::evaluate(&mut lines, params, &adv, None)?;
    Ok(Prepared {
        imported: doc.imported,
        doc_type,
        lines,
        seen_updated_at: doc.updated_at,
        customer,
        bank,
        company: company_repo::get(db, space).await?,
        rate,
        params,
        evaluated,
    })
}

/// The issue validations (422 with every failing field).
async fn check(
    db: &DatabaseConnection,
    space: SpaceId,
    doc: &document::Model,
    doc_type: DocType,
    lines: &[LineData],
) -> Result<(Option<contact::Model>, Option<bank_account::Model>), AppError> {
    let mut e = FieldErrors::new();
    let customer = match doc.contact_id {
        Some(cid) => context::contact(db, space, cid).await?,
        None => None,
    };
    if customer.is_none() && !customer_optional(db, space, doc, doc_type).await? {
        e.add("contactId", "required");
    }
    if !lines.iter().any(|l| matches!(l, LineData::Item(_))) {
        e.add("lines", "required");
    }
    if doc.due_date.is_none_or(|d| d < doc.issue_date) {
        e.add("dueDate", "invalid");
    }
    // Definitions may have changed since the draft was saved.
    let stored = meta::stored_fields(doc);
    let defs = meta::defs(db, space, Direction::Issued).await?;
    custom_fields::validate(&stored, &defs, &stored, &mut e);
    match (doc_type, doc.tax_point_date) {
        (DocType::Proforma, Some(_)) => e.add("taxPointDate", "invalid"),
        (DocType::Proforma, None) | (_, Some(_)) => {}
        (_, None) => e.add("taxPointDate", "required"),
    }
    // Imported corrections document what was issued elsewhere: no reason.
    if doc_type.is_correction() && !doc.imported && doc.correction_reason.is_none() {
        e.add("correctionReason", "required");
    }
    let bank = match doc.bank_account_id {
        Some(bid) => context::bank_account(db, space, bid).await?,
        None => None,
    };
    match &bank {
        Some(b) if b.currency != doc.currency => e.add("bankAccountId", "invalid"),
        None if doc.payment_method == PaymentMethod::BankTransfer => {
            e.add("bankAccountId", "required")
        }
        _ => {}
    }
    e.into_result()?;
    Ok((customer, bank))
}

/// A simplified document may name no customer, and so may a native credit /
/// debit note of one issued without a customer (it is bound to that null).
async fn customer_optional(
    db: &DatabaseConnection,
    space: SpaceId,
    doc: &document::Model,
    doc_type: DocType,
) -> Result<bool, AppError> {
    Ok(match (doc_type, doc.related_document_id) {
        (DocType::Simplified, _) => true,
        (DocType::CreditNote | DocType::DebitNote, Some(orig)) if !doc.imported => {
            let orig = query::find(db, space, orig).await?;
            orig.doc_type == DocType::Simplified && orig.customer_snapshot.is_none()
        }
        _ => false,
    })
}

/// The last 10 digits of the number.
pub fn variable_symbol(number: &str) -> String {
    let digits: Vec<char> = number.chars().filter(char::is_ascii_digit).collect();
    digits[digits.len().saturating_sub(10)..].iter().collect()
}

pub(crate) fn supplier(c: &company::Model) -> PartySnapshot {
    PartySnapshot {
        name: c.name.clone(),
        ico: c.ico.clone(),
        dic: c.dic.clone(),
        street: c.street.clone(),
        city: c.city.clone(),
        zip: c.zip.clone(),
        country: c.country.clone(),
        registration: c.registration.clone(),
        vat_payer: Some(c.vat_payer),
        email: c.email.clone(),
        phone: c.phone.clone(),
        web: c.web.clone(),
    }
}

pub(crate) fn customer(c: &contact::Model) -> PartySnapshot {
    PartySnapshot {
        name: c.name.clone(),
        ico: c.ico.clone(),
        dic: c.dic.clone(),
        street: c.street.clone(),
        city: c.city.clone(),
        zip: c.zip.clone(),
        country: c.country.clone(),
        registration: None,
        vat_payer: None,
        email: c.email.clone(),
        phone: c.phone.clone(),
        web: None,
    }
}

pub(super) fn json<T: serde::Serialize>(v: &T) -> Result<serde_json::Value, AppError> {
    Ok(serde_json::to_value(v).map_err(anyhow::Error::from)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_symbol_keeps_last_ten_digits() {
        assert_eq!(variable_symbol("20260001"), "20260001");
        assert_eq!(variable_symbol("FV-2026/0042"), "20260042");
        assert_eq!(variable_symbol("123456789012"), "3456789012");
    }
}
