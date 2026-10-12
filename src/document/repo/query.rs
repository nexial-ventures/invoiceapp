//! Read paths: one document with its lines and recap, and the filtered list.

use chrono::NaiveDate;
use sea_orm::sea_query::extension::postgres::PgExpr;
use sea_orm::sea_query::{Condition, Expr, NullOrdering, Order, SimpleExpr};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Select,
};
use uuid::Uuid;

use super::lines;
use crate::document::entity::document::{self, Column, Entity};
use crate::document::entity::{document_line, vat_recap};
use crate::document::line::{LineData, PaymentState, Status};
use crate::document::state;
use crate::document::wire::dto::ListQuery;
use crate::error::AppError;
use crate::paging::like_pattern;
use crate::space::SpaceId;

/// A document with its lines (by position), stored recap rows and the
/// documents that reference it (`related_document_id`, oldest first).
pub struct Full {
    pub doc: document::Model,
    pub lines: Vec<LineData>,
    pub recap: Vec<vat_recap::Model>,
    pub related: Vec<document::Model>,
    /// The document `related_document_id` points at.
    pub parent: Option<document::Model>,
    /// Issued DDPP: why it cannot be corrected now.
    pub correction_block: Option<&'static str>,
}

/// The document `id` of `space`; another space's id is not found.
pub async fn find<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    id: Uuid,
) -> Result<document::Model, AppError> {
    Entity::find_by_id(id)
        .filter(Column::SpaceId.eq(space))
        .one(db)
        .await?
        .ok_or(AppError::NotFound)
}

/// The document row, locked until the transaction ends.
pub async fn lock<C: ConnectionTrait>(
    txn: &C,
    space: SpaceId,
    id: Uuid,
) -> Result<document::Model, AppError> {
    Entity::find_by_id(id)
        .filter(Column::SpaceId.eq(space))
        .lock_exclusive()
        .one(txn)
        .await?
        .ok_or(AppError::NotFound)
}

pub async fn load_lines<C: ConnectionTrait>(db: &C, id: Uuid) -> Result<Vec<LineData>, AppError> {
    document_line::Entity::find()
        .filter(document_line::Column::DocumentId.eq(id))
        .order_by_asc(document_line::Column::Position)
        .all(db)
        .await?
        .into_iter()
        .map(lines::to_data)
        .collect()
}

pub async fn load<C: ConnectionTrait>(db: &C, space: SpaceId, id: Uuid) -> Result<Full, AppError> {
    let doc = find(db, space, id).await?;
    let lines = load_lines(db, id).await?;
    let recap = vat_recap::Entity::find()
        .filter(vat_recap::Column::DocumentId.eq(id))
        .order_by_desc(vat_recap::Column::VatRate)
        .all(db)
        .await?;
    let related = Entity::find()
        .filter(Column::SpaceId.eq(space))
        .filter(Column::RelatedDocumentId.eq(id))
        .order_by_asc(Column::CreatedAt)
        .order_by_asc(Column::Id)
        .all(db)
        .await?;
    let parent = match doc.related_document_id {
        Some(pid) => {
            Entity::find_by_id(pid)
                .filter(Column::SpaceId.eq(space))
                .one(db)
                .await?
        }
        None => None,
    };
    let correction_block = super::ddpp_correction::block(db, &doc).await?;
    Ok(Full {
        correction_block,
        doc,
        lines,
        recap,
        related,
        parent,
    })
}

/// Issued and carrying a payment state (a DDPP has none).
fn issued() -> SimpleExpr {
    Column::Status
        .eq(Status::Issued)
        .and(Column::DocType.ne(state::NO_PAYMENTS_DOC_TYPE))
}

/// SQL mirror of [`crate::document::state::payment_state`] (`paid >= 0`).
fn payment_state_cond(state: PaymentState) -> Condition {
    let paid = || Expr::col(Column::Paid);
    let payable = || Expr::col(Column::Payable);
    let cond = match state {
        PaymentState::Unpaid => Condition::all().add(paid().eq(0)).add(payable().gt(0)),
        PaymentState::Partial => Condition::all().add(paid().gt(0)).add(paid().lt(payable())),
        PaymentState::Paid => Condition::any()
            .add(paid().eq(payable()))
            .add(Condition::all().add(paid().eq(0)).add(payable().lt(0))),
        PaymentState::Overpaid => Condition::all().add(paid().gt(payable())).add(paid().gt(0)),
    };
    Condition::all().add(issued()).add(cond)
}

/// SQL mirror of [`crate::document::state::is_overdue`].
fn overdue_cond(today: NaiveDate) -> Condition {
    Condition::all()
        .add(issued())
        .add(Expr::col(Column::Paid).lt(Expr::col(Column::Payable)))
        .add(Column::DueDate.lt(today))
}

fn search(term: &str) -> Condition {
    let pattern = like_pattern(term);
    let live_name = Expr::cust_with_values(
        "customer_snapshot IS NULL AND EXISTS (SELECT 1 FROM contacts c \
         WHERE c.id = documents.contact_id AND c.name ILIKE $1)",
        [pattern.clone()],
    );
    Condition::any()
        .add(Expr::col(Column::Number).ilike(pattern.clone()))
        .add(Expr::col(Column::VariableSymbol).ilike(pattern.clone()))
        .add(Expr::col(Column::SupplierNumber).ilike(pattern.clone()))
        .add(Expr::cust_with_values(
            "customer_snapshot->>'name' ILIKE $1",
            [pattern.clone()],
        ))
        .add(Expr::cust_with_values(
            "direction = 'received' AND supplier_snapshot->>'name' ILIKE $1",
            [pattern],
        ))
        .add(live_name)
}

pub(crate) fn filtered(
    space: SpaceId,
    q: &ListQuery,
    term: Option<&str>,
    today: NaiveDate,
) -> Select<Entity> {
    let mut cond = Condition::all().add(Column::SpaceId.eq(space));
    if let Some(d) = &q.direction {
        cond = cond.add(Column::Direction.eq(d.as_str()));
    }
    if let Some(t) = &q.doc_type {
        cond = cond.add(Column::DocType.eq(t.as_str()));
    }
    if let Some(s) = q.status {
        cond = cond.add(Column::Status.eq(s));
    }
    if let Some(p) = q.payment_state {
        cond = cond.add(payment_state_cond(p));
    }
    match q.overdue {
        Some(true) => cond = cond.add(overdue_cond(today)),
        Some(false) => cond = cond.add(overdue_cond(today).not()),
        None => {}
    }
    if let Some(c) = q.contact_id {
        cond = cond.add(Column::ContactId.eq(c));
    }
    if let Some(c) = q.category_id {
        cond = cond.add(Column::CategoryId.eq(c));
    }
    if let Some(i) = q.imported {
        cond = cond.add(Column::Imported.eq(i));
    }
    if let Some(t) = term {
        cond = cond.add(search(t));
    }
    if let Some(from) = q.from {
        cond = cond.add(Column::IssueDate.gte(from));
    }
    if let Some(to) = q.to {
        cond = cond.add(Column::IssueDate.lte(to));
    }
    Entity::find().filter(cond)
}

/// Every document the list filter `q` matches, unpaged (`limit` / `offset`
/// ignored): the base of the bulk exports (ISDOC, CSV).
pub fn export_select(space: SpaceId, q: &ListQuery, today: NaiveDate) -> Select<Entity> {
    let (term, _, _) = crate::paging::ListQuery {
        q: q.q.clone(),
        limit: None,
        offset: None,
    }
    .normalized();
    filtered(space, q, term.as_deref(), today)
}

/// One page of matching documents and the total match count, ordered
/// `issueDate desc, number desc nulls first, createdAt desc`.
pub async fn list(
    db: &DatabaseConnection,
    space: SpaceId,
    q: &ListQuery,
    term: Option<&str>,
    limit: u64,
    offset: u64,
    today: NaiveDate,
) -> Result<(Vec<document::Model>, u64), AppError> {
    let total = filtered(space, q, term, today).count(db).await?;
    let items = filtered(space, q, term, today)
        .order_by_desc(Column::IssueDate)
        .order_by_with_nulls(Column::Number, Order::Desc, NullOrdering::First)
        .order_by_desc(Column::CreatedAt)
        .order_by_desc(Column::Id)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await?;
    Ok((items, total))
}

/// Highest `number_seq` issued from `doc_type`'s series in `year`
/// (imported documents keep their own numbers and are ignored).
pub async fn highest_issued_seq<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    doc_type: crate::document::doc_type::DocType,
    year: i32,
) -> Result<Option<i32>, AppError> {
    let (direction, doc_type) = doc_type.numbered();
    let max: Option<Option<i32>> = Entity::find()
        .filter(Column::SpaceId.eq(space))
        .select_only()
        .column_as(Column::NumberSeq.max(), "max")
        .filter(Column::Direction.eq(direction))
        .filter(Column::DocType.eq(doc_type))
        .filter(Column::NumberYear.eq(year))
        .filter(Column::Imported.eq(false))
        .into_tuple()
        .one(db)
        .await?;
    Ok(max.flatten())
}
