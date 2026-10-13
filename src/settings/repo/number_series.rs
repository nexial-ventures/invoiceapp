//! Number series of a space (one row per series key) and their yearly
//! counters (children of the series row). Allocation: [`super::numbering`].

use anyhow::Context;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, Set,
};

use crate::document::doc_type::DocType;
use crate::error::AppError;
use crate::settings::entity::{number_series, number_series_counter as counter};
use crate::space::SpaceId;

use super::numbering;
pub use super::numbering::allocate_number;

/// One series with its counters, newest year first.
pub struct Series {
    pub doc_type: DocType,
    pub pattern: String,
    pub client_pattern: Option<String>,
    pub counters: Vec<counter::Model>,
    /// The global series' next free number in `year` (the year it was
    /// loaded for).
    pub next_number_preview: String,
}

/// Every series; the previews for `year`.
pub async fn list<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    year: i32,
) -> Result<Vec<Series>, AppError> {
    let series = number_series::Entity::find()
        .filter(number_series::Column::SpaceId.eq(space))
        .all(db)
        .await?;
    let ids: Vec<_> = series.iter().map(|s| s.id).collect();
    let counters = counter::Entity::find()
        .filter(counter::Column::SeriesId.is_in(ids))
        .order_by_desc(counter::Column::Year)
        .all(db)
        .await?;
    let mut out = Vec::new();
    for doc_type in DocType::ALL {
        let row = series
            .iter()
            .find(|s| s.doc_type == doc_type)
            .with_context(|| format!("number series {} missing", doc_type.as_str()))?;
        let counters = counters
            .iter()
            .filter(|c| c.series_id == row.id)
            .cloned()
            .collect();
        out.push(build(db, space, row, counters, year).await?);
    }
    Ok(out)
}

async fn build<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    row: &number_series::Model,
    counters: Vec<counter::Model>,
    year: i32,
) -> Result<Series, AppError> {
    let last = counters
        .iter()
        .find(|c| c.year == year)
        .map_or(0, |c| c.last_number);
    let scope = numbering::Scope::of(row, None)?;
    Ok(Series {
        doc_type: row.doc_type,
        pattern: row.pattern.clone(),
        client_pattern: row.client_pattern.clone(),
        counters,
        next_number_preview: numbering::preview_in(db, space, &scope, year, last).await?,
    })
}

pub async fn get<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    doc_type: DocType,
    year: i32,
) -> Result<Series, AppError> {
    let row = load(db, space, doc_type).await?;
    let counters = counter::Entity::find()
        .filter(counter::Column::SeriesId.eq(row.id))
        .order_by_desc(counter::Column::Year)
        .all(db)
        .await?;
    build(db, space, &row, counters, year).await
}

/// Change a series' pattern; one already used by another series of the
/// space → `pattern: duplicate`.
pub async fn set_pattern(
    db: &DatabaseConnection,
    space: SpaceId,
    doc_type: DocType,
    pattern: String,
    year: i32,
) -> Result<Series, AppError> {
    let taken = number_series::Entity::find()
        .filter(number_series::Column::SpaceId.eq(space))
        .filter(number_series::Column::DocType.ne(doc_type))
        .filter(number_series::Column::Pattern.eq(pattern.as_str()))
        .one(db)
        .await?;
    if taken.is_some() {
        return Err(AppError::field("pattern", "duplicate"));
    }
    let row = load(db, space, doc_type).await?;
    let mut row: number_series::ActiveModel = row.into();
    row.pattern = Set(pattern);
    row.update(db).await?;
    get(db, space, doc_type, year).await
}

/// Set (`Some`, already validated) or clear an issued series' client
/// pattern; one already used by another series of the space →
/// `clientPattern: duplicate`.
pub async fn set_client_pattern(
    db: &DatabaseConnection,
    space: SpaceId,
    doc_type: DocType,
    client_pattern: Option<String>,
    year: i32,
) -> Result<Series, AppError> {
    if let Some(p) = &client_pattern {
        let taken = number_series::Entity::find()
            .filter(number_series::Column::SpaceId.eq(space))
            .filter(number_series::Column::DocType.ne(doc_type))
            .filter(number_series::Column::ClientPattern.eq(p.as_str()))
            .one(db)
            .await?;
        if taken.is_some() {
            return Err(AppError::field("clientPattern", "duplicate"));
        }
    }
    let row = load(db, space, doc_type).await?;
    let mut row: number_series::ActiveModel = row.into();
    row.client_pattern = Set(client_pattern);
    row.update(db).await?;
    get(db, space, doc_type, year).await
}

/// Set a counter by hand, to any value: allocation takes the next free
/// number after it, so a low counter fills gaps and never repeats a number.
pub async fn set_counter(
    db: &DatabaseConnection,
    space: SpaceId,
    doc_type: DocType,
    year: i32,
    last_number: i32,
    preview_year: i32,
) -> Result<Series, AppError> {
    let series_id = load(db, space, doc_type).await?.id;
    counter::Entity::insert(counter::ActiveModel {
        series_id: Set(series_id),
        year: Set(year),
        last_number: Set(last_number),
    })
    .on_conflict(
        OnConflict::columns([counter::Column::SeriesId, counter::Column::Year])
            .update_column(counter::Column::LastNumber)
            .to_owned(),
    )
    .exec(db)
    .await?;
    get(db, space, doc_type, preview_year).await
}

pub(crate) async fn load<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    doc_type: DocType,
) -> Result<number_series::Model, AppError> {
    let row = number_series::Entity::find()
        .filter(number_series::Column::SpaceId.eq(space))
        .filter(number_series::Column::DocType.eq(doc_type))
        .one(db)
        .await?
        .with_context(|| format!("number series {} missing", doc_type.as_str()))?;
    Ok(row)
}
