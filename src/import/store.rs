//! Confirm: one planned entry → one stored document, in its own transaction
//! (contact, category, document, lines, recap, original PDF, payment).

use bytes::Bytes;
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    EntityTrait, QueryFilter, Set, Statement, TransactionTrait,
};
use uuid::Uuid;

use super::category::{self, CategoryRef};
use super::lookup;
use super::model::{ContactRule, IssuedBank, Party, Plan};
use crate::contact::entity::contact;
use crate::document::entity::{document, payment};
use crate::document::handlers::dto::BankSnapshot;
use crate::document::line::{RoundingMode, Status};
use crate::document::repo::issue::Rate;
use crate::document::repo::{lines, original, payments, write};
use crate::error::{AppError, number_violation};
use crate::pdf::PdfService;
use crate::pdf::editor::ensure_usage;
use crate::settings::doc_type::{DocType, ISSUED, RECEIVED};
use crate::settings::entity::bank_account;
use crate::settings::repo::number_series;
use crate::space::{SpaceId, limits};
use crate::validation::normalize_iban;

/// What a source decides per entry beyond the plan.
#[derive(Debug, Clone)]
pub struct Options {
    /// One payment of the payable on this date (not for a DDPP, nor when
    /// the payable is 0).
    pub paid_on: Option<NaiveDate>,
    pub category: Option<CategoryRef>,
    /// Received only.
    pub vat_deductible: bool,
}

fn json<T: serde::Serialize>(v: &T) -> Result<serde_json::Value, AppError> {
    Ok(serde_json::to_value(v).map_err(anyhow::Error::from)?)
}

async fn contact_for<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    p: &Party,
    rule: ContactRule,
) -> Result<Uuid, AppError> {
    if let Some(c) = lookup::contact(db, space, p, rule).await? {
        return Ok(c.id);
    }
    let now = chrono::Utc::now().into();
    let id = Uuid::new_v4();
    contact::ActiveModel {
        id: Set(id),
        space_id: Set(space.uuid()),
        name: Set(p.name.clone()),
        ico: Set(p.ico.clone()),
        dic: Set(p.dic.clone()),
        street: Set(p.street.clone()),
        city: Set(p.city.clone()),
        zip: Set(p.zip.clone()),
        country: Set(p.country.clone()),
        email: Set(p.email.clone()),
        phone: Set(p.phone.clone()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(id)
}

/// Our bank account named by the payment details.
async fn matching_account<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    bank: &BankSnapshot,
) -> Result<Option<Uuid>, AppError> {
    let wanted = [bank.iban.as_deref(), bank.account_number.as_deref()];
    let wanted: Vec<String> = wanted.into_iter().flatten().map(normalize_iban).collect();
    Ok(bank_account::Entity::find()
        .filter(bank_account::Column::SpaceId.eq(space))
        .all(db)
        .await?
        .into_iter()
        .find(|b| {
            [b.iban.as_deref(), b.account_number.as_deref()]
                .into_iter()
                .flatten()
                .any(|x| wanted.contains(&normalize_iban(x)))
        })
        .map(|b| b.id))
}

/// An issued document's bank account and snapshot.
async fn issued_bank<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    plan: &Plan,
) -> Result<(Option<Uuid>, Option<BankSnapshot>), AppError> {
    match plan.issued_bank {
        IssuedBank::Payment => match &plan.bank {
            Some(b) => Ok((matching_account(db, space, b).await?, Some(b.clone()))),
            None => Ok((None, None)),
        },
        IssuedBank::CurrencyDefault => {
            let account = bank_account::Entity::find()
                .filter(bank_account::Column::SpaceId.eq(space))
                .filter(bank_account::Column::Currency.eq(plan.currency.as_str()))
                .filter(bank_account::Column::IsDefault.eq(true))
                .one(db)
                .await?;
            Ok(match account {
                Some(a) => (
                    Some(a.id),
                    Some(BankSnapshot {
                        account_number: a.account_number,
                        iban: a.iban,
                        bic: a.bic,
                    }),
                ),
                None => (None, None),
            })
        }
    }
}

/// Payable types; a DDPP takes no payment.
fn payable(t: DocType) -> bool {
    t != DocType::AdvanceTaxDoc
}

async fn insert(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
    plan: &Plan,
    file: Option<&original::Original>,
    rate: &Rate,
    opts: &Options,
) -> Result<(), AppError> {
    // Serialise imports of the same document and re-check under the lock: no
    // unique index covers received documents (supplier + supplier number).
    txn.query_one(Statement::from_sql_and_values(
        txn.get_database_backend(),
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        [format!("{space}|{}", lookup::identity(plan)).into()],
    ))
    .await?;
    if lookup::duplicate(txn, space, plan).await? {
        return Err(AppError::Duplicate);
    }
    let now = chrono::Utc::now();
    let issued = plan.direction == ISSUED;
    let contact_id = match plan.counterparty() {
        Some(p) => Some(contact_for(txn, space, p, plan.contact_rule).await?),
        None => None,
    };
    let related = lookup::related(txn, space, plan).await?;
    let received_date = plan.received_date();
    let category_id = match &opts.category {
        Some(c) => category::resolve(txn, space, plan.direction, c).await?,
        None => None,
    };
    let (bank_account_id, bank_snapshot) = if issued {
        issued_bank(txn, space, plan).await?
    } else {
        (None, None)
    };
    let year = year(plan);
    let (number, seq) = if issued {
        (plan.number.clone(), None)
    } else {
        let (n, seq) =
            number_series::allocate_number(txn, space, plan.doc_type.series(RECEIVED), year)
                .await?;
        (n, Some(seq))
    };
    let mut row = document::ActiveModel {
        id: Set(id),
        space_id: Set(space.uuid()),
        direction: Set(plan.direction.into()),
        doc_type: Set(plan.doc_type.as_str().into()),
        status: Set(Status::Issued.as_str().into()),
        number: Set(Some(number)),
        number_year: Set(Some(year)),
        number_seq: Set(seq),
        imported: Set(issued),
        contact_id: Set(contact_id),
        issue_date: Set(plan.issue_date),
        tax_point_date: Set(plan.tax_point_date),
        due_date: Set(plan.due_date),
        currency: Set(plan.currency.clone()),
        exchange_rate: Set(rate.rate),
        exchange_rate_date: Set(rate.date),
        exchange_rate_source: Set(rate.source.map(str::to_string)),
        locale: Set(plan.locale.clone()),
        vat_mode: Set(plan.vat_mode.as_str().into()),
        bank_account_id: Set(bank_account_id),
        payment_method: Set(plan.payment_method.as_str().into()),
        variable_symbol: Set(plan.variable_symbol.clone()),
        constant_symbol: Set(plan.constant_symbol.clone()),
        header_note: Set(plan.note.clone().filter(|_| issued)),
        internal_note: Set(plan.note.clone().filter(|_| !issued)),
        // The rounding amount is kept as given; the mode is derived from it.
        rounding_mode: Set(
            RoundingMode::derive(plan.totals.total, plan.totals.rounding)
                .as_str()
                .into(),
        ),
        supplier_snapshot: Set(Some(json(&plan.supplier.snapshot(plan.vat_payer))?)),
        customer_snapshot: Set(match (&plan.customer, issued) {
            (Some(c), true) => Some(json(&c.snapshot(None))?),
            _ => None,
        }),
        bank_snapshot: Set(bank_snapshot.as_ref().map(json).transpose()?),
        paid: Set(Decimal::ZERO),
        related_document_id: Set(related),
        supplier_number: Set((!issued).then(|| plan.number.clone())),
        received_date: Set((!issued).then_some(received_date)),
        vat_deductible: Set(!issued && opts.vat_deductible),
        supplier_account: Set(match (&plan.bank, issued) {
            (Some(b), false) => b
                .iban
                .clone()
                .or_else(|| b.account_number.clone())
                .map(|a| a.chars().take(60).collect()),
            _ => None,
        }),
        category_id: Set(category_id),
        custom_fields: Set(serde_json::json!({})),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };
    write::apply_totals(&mut row, &plan.totals);
    if let Some(f) = file {
        f.apply(&mut row, now);
    }
    row.insert(txn).await.map_err(|e| {
        number_violation(e, || {
            if issued {
                AppError::Duplicate
            } else {
                AppError::NumberTaken
            }
        })
    })?;
    // Display-only for both directions: the recap and totals above come from
    // the source and are never recomputed from these lines.
    lines::replace(txn, id, &plan.lines).await?;
    lines::replace_recap(txn, id, &plan.totals).await?;
    let paid = plan.totals.payable;
    if let Some(date) = opts.paid_on
        && payable(plan.doc_type)
        && paid > Decimal::ZERO
    {
        payment::ActiveModel {
            id: Set(Uuid::new_v4()),
            document_id: Set(id),
            date: Set(date),
            amount: Set(paid),
            note: Set(None),
            created_at: Set(now.into()),
        }
        .insert(txn)
        .await?;
        payments::resum(txn, id).await?;
    }
    Ok(())
}

/// The number year of the stored document (also the year of its original's key).
fn year(plan: &Plan) -> i32 {
    if plan.direction == ISSUED {
        plan.issue_date.year()
    } else {
        plan.received_date().year()
    }
}

/// Store one document: the PDF first (own per-attempt key, so no row lock
/// is held across the storage call), then the rows in one transaction. A
/// failed transaction removes the written PDF again (unless a row points at
/// it).
pub async fn import(
    db: &DatabaseConnection,
    pdf: &PdfService,
    plan: &Plan,
    file: Option<Bytes>,
    rate: &Rate,
    opts: &Options,
) -> Result<Uuid, AppError> {
    let space = pdf.space_id();
    let id = Uuid::new_v4();
    let stored = match file {
        Some(bytes) => {
            ensure_usage(db, space, pdf.storage()).await?;
            let f = original::Original::new(year(plan), id, &bytes)?;
            pdf.storage().put(&f.rel, bytes).await?;
            Some(f)
        }
        None => None,
    };
    let added = stored
        .as_ref()
        .map_or(0, |f| u64::try_from(f.size).unwrap_or(0));
    let txn = match db.begin().await {
        Ok(txn) => txn,
        Err(e) => {
            if let Some(f) = &stored {
                pdf.storage().remove(&f.rel).await;
            }
            return Err(e.into());
        }
    };
    let result = async {
        insert(&txn, space, id, plan, stored.as_ref(), rate, opts).await?;
        // Last, under the space's quota lock; a refusal removes the object.
        limits::check_documents(&txn, space, added, 0).await
    }
    .await;
    let result = match result {
        Ok(()) => txn.commit().await.map_err(AppError::from),
        Err(e) => Err(rollback(txn, e).await),
    };
    if let Err(e) = result {
        if let Some(f) = &stored {
            original::remove_unreferenced(db, pdf, &f.rel).await;
        }
        return Err(e);
    }
    Ok(id)
}

async fn rollback(txn: DatabaseTransaction, e: AppError) -> AppError {
    if let Err(r) = txn.rollback().await {
        tracing::warn!(error = %r, "rollback import entry");
    }
    e
}
