//! The category an imported document gets: a given id (ISDOC batch option)
//! or a name matched within the kind of the direction, created when missing
//! (CSV).

use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Statement,
};
use uuid::Uuid;

use crate::document::meta::category_kind;
use crate::error::AppError;
use crate::settings::entity::category::{self, Column, Entity};
use crate::space::SpaceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryRef {
    Id(Uuid),
    Name(String),
}

/// The category of `kind` named `name` (case-insensitive, trimmed).
pub async fn find<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    kind: &str,
    name: &str,
) -> Result<Option<category::Model>, AppError> {
    Ok(Entity::find()
        .filter(Column::SpaceId.eq(space))
        .filter(Column::Kind.eq(kind))
        .filter(Expr::cust_with_values(
            "lower(name) = lower($1)",
            [name.trim().to_string()],
        ))
        .one(db)
        .await?)
}

/// The category id to store: an id as given; a name → the matching active
/// category (an inactive one → none), created (active, appended last) when
/// there is none.
pub async fn resolve<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    direction: &str,
    r: &CategoryRef,
) -> Result<Option<Uuid>, AppError> {
    let name = match r {
        // Checked against the space when the batch options were read.
        CategoryRef::Id(id) => return Ok(Some(*id)),
        CategoryRef::Name(name) => name.trim(),
    };
    let kind = category_kind(direction);
    if let Some(c) = find(db, space, kind, name).await? {
        return Ok(c.active.then_some(c.id));
    }
    let last = Entity::find()
        .filter(Column::SpaceId.eq(space))
        .filter(Column::Kind.eq(kind))
        .order_by_desc(Column::Position)
        .select_only()
        .column(Column::Position)
        .into_tuple::<i32>()
        .one(db)
        .await?;
    // A concurrent import may create the same category: keep theirs.
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO categories (id, space_id, name, kind, active, position, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, true, $5, now(), now()) ON CONFLICT DO NOTHING",
        [
            Uuid::new_v4().into(),
            space.into(),
            name.into(),
            kind.into(),
            last.map_or(0, |p| p.saturating_add(1)).into(),
        ],
    ))
    .await?;
    Ok(find(db, space, kind, name)
        .await?
        .filter(|c| c.active)
        .map(|c| c.id))
}
