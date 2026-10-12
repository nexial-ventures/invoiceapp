use sea_orm::sea_query::Expr;
use sea_orm::sea_query::extension::postgres::PgExpr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait,
};
use uuid::Uuid;

use crate::catalog::entity::item::{self, ActiveModel, Column, Entity};
use crate::catalog::entity::member;
use crate::catalog::handlers::dto::ItemData;
use crate::error::AppError;
use crate::paging::like_pattern;
use crate::space::SpaceId;

/// Items matching `q` (name, case-insensitive) and `active`, by name.
pub async fn list(
    db: &DatabaseConnection,
    space: SpaceId,
    q: Option<&str>,
    active: Option<bool>,
) -> Result<Vec<item::Model>, AppError> {
    let mut select = Entity::find().filter(Column::SpaceId.eq(space));
    if let Some(term) = q {
        select = select.filter(Expr::col(Column::Name).ilike(like_pattern(term)));
    }
    if let Some(a) = active {
        select = select.filter(Column::Active.eq(a));
    }
    Ok(select
        .order_by_asc(Column::Name)
        .order_by_asc(Column::Id)
        .all(db)
        .await?)
}

pub async fn get(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
) -> Result<item::Model, AppError> {
    Entity::find_by_id(id)
        .filter(Column::SpaceId.eq(space))
        .one(db)
        .await?
        .ok_or(AppError::NotFound)
}

fn apply(row: &mut ActiveModel, d: ItemData) {
    row.name = Set(d.name);
    row.unit = Set(d.unit);
    row.unit_price = Set(d.unit_price);
    row.currency = Set(d.currency);
    row.vat_rate = Set(d.vat_rate);
    row.active = Set(d.active);
    row.note = Set(d.note);
}

pub async fn create(
    db: &DatabaseConnection,
    space: SpaceId,
    data: ItemData,
) -> Result<item::Model, AppError> {
    let now = chrono::Utc::now().into();
    let mut row = ActiveModel {
        id: Set(Uuid::new_v4()),
        space_id: Set(space.uuid()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    apply(&mut row, data);
    Ok(row.insert(db).await?)
}

/// Members of every group `item_id` belongs to (the item's own rows included).
async fn group_mates<C: ConnectionTrait>(
    db: &C,
    item_id: Uuid,
) -> Result<Vec<member::Model>, AppError> {
    let groups: Vec<Uuid> = member::Entity::find()
        .filter(member::Column::ItemId.eq(item_id))
        .all(db)
        .await?
        .into_iter()
        .map(|m| m.group_id)
        .collect();
    Ok(member::Entity::find()
        .filter(member::Column::GroupId.is_in(groups))
        .all(db)
        .await?)
}

/// The item row locked against concurrent group writes (which share-lock it).
async fn locked<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    id: Uuid,
) -> Result<item::Model, AppError> {
    Entity::find_by_id(id)
        .filter(Column::SpaceId.eq(space))
        .lock_exclusive()
        .one(db)
        .await?
        .ok_or(AppError::NotFound)
}

/// A new VAT rate may not make any of the item's groups mixed
/// (`vatRate: mixed_vat`).
pub async fn update(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    data: ItemData,
) -> Result<item::Model, AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let existing = locked(txn, space, id).await?;
                if existing.vat_rate != data.vat_rate {
                    let others: Vec<Uuid> = group_mates(txn, id)
                        .await?
                        .into_iter()
                        .map(|m| m.item_id)
                        .filter(|i| *i != id)
                        .collect();
                    let mixed = Entity::find()
                        .filter(Column::Id.is_in(others))
                        .filter(Column::VatRate.ne(data.vat_rate))
                        .one(txn)
                        .await?
                        .is_some();
                    if mixed {
                        return Err(AppError::field("vatRate", "mixed_vat"));
                    }
                }
                let mut row: ActiveModel = existing.into();
                apply(&mut row, data);
                row.updated_at = Set(chrono::Utc::now().into());
                Ok(row.update(txn).await?)
            })
        })
        .await?)
}

/// Group memberships go with the item (FK cascade), except that a group may
/// not lose its last member (`catalog_item_in_use`).
pub async fn delete(db: &DatabaseConnection, space: SpaceId, id: Uuid) -> Result<(), AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                locked(txn, space, id).await?;
                let mates = group_mates(txn, id).await?;
                let sole_member = mates
                    .iter()
                    .filter(|m| m.item_id == id)
                    .any(|m| mates.iter().filter(|o| o.group_id == m.group_id).count() == 1);
                if sole_member {
                    return Err(AppError::CatalogItemInUse);
                }
                Entity::delete_by_id(id).exec(txn).await?;
                Ok(())
            })
        })
        .await?)
}
