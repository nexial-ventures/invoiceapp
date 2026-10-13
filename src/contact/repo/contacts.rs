use sea_orm::sea_query::extension::postgres::PgExpr;
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Select, Set,
};
use uuid::Uuid;

use crate::contact::entity::contact::{self, ActiveModel, Column, Entity};
use crate::contact::handlers::dto::ContactInput;
use crate::error::{AppError, unique_violation};
use crate::paging::like_pattern;
use crate::space::SpaceId;

/// One page of contacts matching `q` (ordered by name) and the total match count.
pub async fn list(
    db: &DatabaseConnection,
    space: SpaceId,
    q: Option<&str>,
    limit: u64,
    offset: u64,
) -> Result<(Vec<contact::Model>, u64), AppError> {
    let total = search(space, q).count(db).await?;
    let items = search(space, q)
        .order_by_asc(Column::Name)
        .order_by_asc(Column::Id)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await?;
    Ok((items, total))
}

fn search(space: SpaceId, q: Option<&str>) -> Select<Entity> {
    let scoped = Entity::find().filter(Column::SpaceId.eq(space));
    let Some(term) = q else {
        return scoped;
    };
    let pattern = like_pattern(term);
    let cond = [
        Column::Name,
        Column::Ico,
        Column::Dic,
        Column::City,
        Column::NumberCode,
    ]
    .into_iter()
    .fold(Condition::any(), |c, col| {
        c.add(Expr::col(col).ilike(pattern.clone()))
    });
    scoped.filter(cond)
}

pub async fn get(
    db: &impl sea_orm::ConnectionTrait,
    space: SpaceId,
    id: Uuid,
) -> Result<contact::Model, AppError> {
    Entity::find_by_id(id)
        .filter(Column::SpaceId.eq(space))
        .one(db)
        .await?
        .ok_or(AppError::NotFound)
}

/// `input` must already be validated.
pub async fn create(
    db: &DatabaseConnection,
    space: SpaceId,
    input: ContactInput,
) -> Result<contact::Model, AppError> {
    let now = chrono::Utc::now().into();
    let mut row = active(input);
    row.id = Set(Uuid::new_v4());
    row.space_id = Set(space.uuid());
    row.created_at = Set(now);
    row.updated_at = Set(now);
    row.insert(db).await.map_err(map_duplicate)
}

/// `input` must already be validated; replaces every editable field.
pub async fn update(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    input: ContactInput,
) -> Result<contact::Model, AppError> {
    let existing = get(db, space, id).await?;
    let mut row = active(input);
    row.id = Set(id);
    row.space_id = Set(space.uuid());
    row.created_at = Set(existing.created_at);
    row.updated_at = Set(chrono::Utc::now().into());
    row.update(db).await.map_err(map_duplicate)
}

pub async fn delete(db: &DatabaseConnection, space: SpaceId, id: Uuid) -> Result<(), AppError> {
    let res = Entity::delete_many()
        .filter(Column::Id.eq(id))
        .filter(Column::SpaceId.eq(space))
        .exec(db)
        .await?;
    if res.rows_affected == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

fn active(c: ContactInput) -> ActiveModel {
    ActiveModel {
        name: Set(c.name),
        ico: Set(c.ico),
        dic: Set(c.dic),
        street: Set(c.street),
        city: Set(c.city),
        zip: Set(c.zip),
        country: Set(c.country),
        email: Set(c.email),
        phone: Set(c.phone),
        note: Set(c.note),
        default_due_days: Set(c.default_due_days),
        default_locale: Set(c.default_locale),
        default_currency: Set(c.default_currency),
        number_code: Set(c.number_code),
        ..Default::default()
    }
}

fn map_duplicate(err: DbErr) -> AppError {
    match unique_violation(&err) {
        Some(constraint) if constraint == "contacts_ico_key" => AppError::field("ico", "duplicate"),
        Some(constraint) if constraint == "contacts_number_code_key" => {
            AppError::field("numberCode", "duplicate")
        }
        _ => err.into(),
    }
}
