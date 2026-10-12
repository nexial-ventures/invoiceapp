//! Payments of an issued document; `documents.paid` is re-summed in the same
//! transaction so list filters on payment state stay exact. A payment on a VAT
//! payer's proforma issues its DDPP in that transaction.

use std::collections::HashMap;

use chrono::NaiveDate;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    EntityTrait, QueryFilter, QueryOrder, Set, Statement, TransactionTrait,
};
use uuid::Uuid;

use super::{ddpp, query, view};
use crate::cnb::CnbClient;
use crate::document::entity::document;
use crate::document::entity::payment::{self, ActiveModel, Column, Entity};
use crate::document::line::{MAX_AMOUNT, Status};
use crate::document::model::NewPayment;
use crate::document::state::NO_PAYMENTS_DOC_TYPE;
use crate::document::wire::dto::Payment;
use crate::error::AppError;
use crate::space::SpaceId;

/// A payment and the DDPP it created, if any.
pub type WithAdvance = (payment::Model, Option<Uuid>);

// Here rather than in `wire`: it reads the payment entity, which `wire` never imports.
impl From<WithAdvance> for Payment {
    fn from((m, advance): WithAdvance) -> Self {
        Self {
            id: m.id,
            date: m.date,
            amount: crate::document::compute::round2(m.amount),
            note: m.note,
            advance_document_id: advance,
            created_at: m.created_at,
        }
    }
}

pub async fn list(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
) -> Result<Vec<WithAdvance>, AppError> {
    query::find(db, space, id).await?;
    let rows = Entity::find()
        .filter(Column::DocumentId.eq(id))
        .order_by_asc(Column::Date)
        .order_by_asc(Column::CreatedAt)
        .all(db)
        .await?;
    let ids: Vec<Uuid> = rows.iter().map(|p| p.id).collect();
    let ddpps: HashMap<Uuid, Uuid> = document::Entity::find()
        .filter(document::Column::PaymentId.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .filter_map(|d| d.payment_id.map(|p| (p, d.id)))
        .collect();
    Ok(rows
        .into_iter()
        .map(|p| {
            let ddpp = ddpps.get(&p.id).copied();
            (p, ddpp)
        })
        .collect())
}

pub async fn create(
    db: &DatabaseConnection,
    cnb: &CnbClient,
    space: SpaceId,
    id: Uuid,
    p: NewPayment,
    today: NaiveDate,
) -> Result<WithAdvance, AppError> {
    let doc = query::find(db, space, id).await?;
    // Resolved up front: no ČNB call while the row is locked, and no payment
    // stored when the DDPP cannot get a rate.
    let rate = if ddpp::needed(&doc) {
        Some(ddpp::rate(db, cnb, &doc, p.exchange_rate, p.date, today).await?)
    } else {
        None
    };
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked_issued(txn, space, id).await?;
                // `documents.paid` is numeric(18,2) too: the new sum must fit.
                if doc
                    .paid
                    .checked_add(p.amount)
                    .is_none_or(|sum| sum >= MAX_AMOUNT)
                {
                    return Err(AppError::field("amount", "invalid"));
                }
                let row = ActiveModel {
                    id: Set(Uuid::new_v4()),
                    document_id: Set(id),
                    date: Set(p.date),
                    amount: Set(p.amount),
                    note: Set(p.note),
                    created_at: Set(chrono::Utc::now().into()),
                }
                .insert(txn)
                .await?;
                resum(txn, id).await?;
                let advance = match (ddpp::needed(&doc), rate) {
                    (true, Some(rate)) => Some(ddpp::issue(txn, &doc, &row, rate).await?),
                    (true, None) => {
                        return Err(AppError::Conflict("document changed meanwhile".into()));
                    }
                    (false, _) => None,
                };
                Ok((row, advance))
            })
        })
        .await?)
}

/// Deleting a proforma payment cancels its DDPP; refused while that DDPP — or,
/// non-payer form, the proforma itself — is deducted by an invoice
/// (`advance_settled` when issued, `advance_in_use` for a draft).
pub async fn delete(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    payment_id: Uuid,
) -> Result<(), AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = locked_issued(txn, space, id).await?;
                let exists = Entity::find_by_id(payment_id)
                    .filter(Column::DocumentId.eq(id))
                    .one(txn)
                    .await?
                    .is_some();
                if !exists {
                    return Err(AppError::NotFound);
                }
                if doc.doc_type == "proforma" {
                    ddpp::ensure_not_deducted(txn, id).await?;
                    ddpp::cancel_for_payment(txn, payment_id).await?;
                }
                Entity::delete_by_id(payment_id).exec(txn).await?;
                resum(txn, id).await
            })
        })
        .await?)
}

/// Lock the row; payments need an issued document that is not a DDPP.
async fn locked_issued(
    txn: &DatabaseTransaction,
    space: SpaceId,
    id: Uuid,
) -> Result<document::Model, AppError> {
    let doc = query::lock(txn, space, id).await?;
    if view::status(&doc)? != Status::Issued || doc.doc_type == NO_PAYMENTS_DOC_TYPE {
        return Err(AppError::InvalidState);
    }
    Ok(doc)
}

pub(crate) async fn resum(txn: &DatabaseTransaction, id: Uuid) -> Result<(), AppError> {
    txn.execute(Statement::from_sql_and_values(
        txn.get_database_backend(),
        "UPDATE documents SET paid = (SELECT COALESCE(SUM(amount), 0) FROM payments \
         WHERE document_id = $1), updated_at = now() WHERE id = $1",
        [id.into()],
    ))
    .await?;
    Ok(())
}
