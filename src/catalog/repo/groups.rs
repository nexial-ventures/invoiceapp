//! Catalog groups with their ordered members. Writes replace the member set
//! in one transaction, with the member items locked (`FOR SHARE`) so a
//! concurrent item delete cannot slip in between the check and the insert.

use std::collections::HashMap;

use sea_orm::sea_query::Expr;
use sea_orm::sea_query::extension::postgres::PgExpr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
};
use uuid::Uuid;

use crate::catalog::entity::{group, item, member};
use crate::catalog::handlers::dto::{GroupData, check_items};
use crate::error::AppError;
use crate::paging::like_pattern;
use crate::space::SpaceId;

/// A group with its members (by position) and their items.
pub struct Full {
    pub group: group::Model,
    pub members: Vec<(member::Model, item::Model)>,
}

async fn with_members<C: ConnectionTrait>(
    db: &C,
    groups: Vec<group::Model>,
) -> Result<Vec<Full>, AppError> {
    let ids: Vec<Uuid> = groups.iter().map(|g| g.id).collect();
    let members = member::Entity::find()
        .filter(member::Column::GroupId.is_in(ids))
        .order_by_asc(member::Column::Position)
        .all(db)
        .await?;
    let item_ids: Vec<Uuid> = members.iter().map(|m| m.item_id).collect();
    let items: HashMap<Uuid, item::Model> = item::Entity::find()
        .filter(item::Column::Id.is_in(item_ids))
        .all(db)
        .await?
        .into_iter()
        .map(|i| (i.id, i))
        .collect();
    Ok(groups
        .into_iter()
        .map(|g| Full {
            members: members
                .iter()
                .filter(|m| m.group_id == g.id)
                .filter_map(|m| items.get(&m.item_id).map(|i| (m.clone(), i.clone())))
                .collect(),
            group: g,
        })
        .collect())
}

pub async fn list(
    db: &DatabaseConnection,
    space: SpaceId,
    q: Option<&str>,
) -> Result<Vec<Full>, AppError> {
    let mut select = group::Entity::find().filter(group::Column::SpaceId.eq(space));
    if let Some(term) = q {
        select = select.filter(Expr::col(group::Column::Name).ilike(like_pattern(term)));
    }
    let groups = select
        .order_by_asc(group::Column::Name)
        .order_by_asc(group::Column::Id)
        .all(db)
        .await?;
    with_members(db, groups).await
}

pub async fn get<C: ConnectionTrait>(db: &C, space: SpaceId, id: Uuid) -> Result<Full, AppError> {
    let g = group::Entity::find_by_id(id)
        .filter(group::Column::SpaceId.eq(space))
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    with_members(db, vec![g])
        .await?
        .pop()
        .ok_or(AppError::NotFound)
}

/// Check the member items (locked) and replace the group's members.
async fn write_members(
    txn: &DatabaseTransaction,
    space: SpaceId,
    group_id: Uuid,
    members: &[(Uuid, rust_decimal::Decimal)],
) -> Result<(), AppError> {
    let ids: Vec<Uuid> = members.iter().map(|(id, _)| *id).collect();
    let items: HashMap<Uuid, item::Model> = item::Entity::find()
        .filter(item::Column::Id.is_in(ids))
        .filter(item::Column::SpaceId.eq(space))
        .lock_shared()
        .all(txn)
        .await?
        .into_iter()
        .map(|i| (i.id, i))
        .collect();
    check_items(members, &items).into_result()?;
    member::Entity::delete_many()
        .filter(member::Column::GroupId.eq(group_id))
        .exec(txn)
        .await?;
    let rows = members
        .iter()
        .zip(1..)
        .map(|((item_id, quantity), position)| member::ActiveModel {
            group_id: Set(group_id),
            item_id: Set(*item_id),
            position: Set(position),
            quantity: Set(*quantity),
        });
    member::Entity::insert_many(rows).exec(txn).await?;
    Ok(())
}

pub async fn create(
    db: &DatabaseConnection,
    space: SpaceId,
    data: GroupData,
) -> Result<Full, AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let now = chrono::Utc::now().into();
                let row = group::ActiveModel {
                    id: Set(Uuid::new_v4()),
                    space_id: Set(space.uuid()),
                    name: Set(data.name),
                    collapse: Set(data.collapse),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(txn)
                .await?;
                write_members(txn, space, row.id, &data.members).await?;
                get(txn, space, row.id).await
            })
        })
        .await?)
}

pub async fn update(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    data: GroupData,
) -> Result<Full, AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let existing = group::Entity::find_by_id(id)
                    .filter(group::Column::SpaceId.eq(space))
                    .lock_exclusive()
                    .one(txn)
                    .await?
                    .ok_or(AppError::NotFound)?;
                let mut row: group::ActiveModel = existing.into();
                row.name = Set(data.name);
                row.collapse = Set(data.collapse);
                row.updated_at = Set(chrono::Utc::now().into());
                row.update(txn).await?;
                write_members(txn, space, id, &data.members).await?;
                get(txn, space, id).await
            })
        })
        .await?)
}

pub async fn delete(db: &DatabaseConnection, space: SpaceId, id: Uuid) -> Result<(), AppError> {
    let res = group::Entity::delete_many()
        .filter(group::Column::Id.eq(id))
        .filter(group::Column::SpaceId.eq(space))
        .exec(db)
        .await?;
    if res.rows_affected == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}
