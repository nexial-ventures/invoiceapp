//! Number series of a space (one row per series key) and their yearly
//! counters (children of the series row).

use anyhow::Context;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set, Statement, TransactionTrait,
};

use crate::document::doc_type::DocType;
use crate::error::AppError;
use crate::settings::entity::{number_series, number_series_counter as counter};
use crate::settings::pattern::Pattern;
use crate::space::SpaceId;

/// One series with its counters, newest year first.
pub struct Series {
    pub doc_type: DocType,
    pub pattern: String,
    pub counters: Vec<counter::Model>,
}

pub async fn list<C: ConnectionTrait>(db: &C, space: SpaceId) -> Result<Vec<Series>, AppError> {
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
    DocType::ALL
        .into_iter()
        .map(|doc_type| {
            let row = series
                .iter()
                .find(|s| s.doc_type == doc_type)
                .with_context(|| format!("number series {} missing", doc_type.as_str()))?;
            let counters = counters
                .iter()
                .filter(|c| c.series_id == row.id)
                .cloned()
                .collect();
            Ok(Series {
                doc_type,
                pattern: row.pattern.clone(),
                counters,
            })
        })
        .collect()
}

pub async fn get<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    doc_type: DocType,
) -> Result<Series, AppError> {
    let row = load(db, space, doc_type).await?;
    let counters = counter::Entity::find()
        .filter(counter::Column::SeriesId.eq(row.id))
        .order_by_desc(counter::Column::Year)
        .all(db)
        .await?;
    Ok(Series {
        doc_type,
        pattern: row.pattern,
        counters,
    })
}

/// Change a series' pattern; one already used by another series of the
/// space → `pattern: duplicate`.
pub async fn set_pattern(
    db: &DatabaseConnection,
    space: SpaceId,
    doc_type: DocType,
    pattern: String,
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
    get(db, space, doc_type).await
}

/// Set a counter by hand; below the highest number issued from the series
/// that year → `lastNumber: below_issued`.
pub async fn set_counter(
    db: &DatabaseConnection,
    space: SpaceId,
    doc_type: DocType,
    year: i32,
    last_number: i32,
) -> Result<Series, AppError> {
    db.transaction(|txn| Box::pin(set_counter_in(txn, space, doc_type, year, last_number)))
        .await?;
    get(db, space, doc_type).await
}

async fn set_counter_in(
    txn: &DatabaseTransaction,
    space: SpaceId,
    doc_type: DocType,
    year: i32,
    last_number: i32,
) -> Result<(), AppError> {
    let series_id = load(txn, space, doc_type).await?.id;
    // Lock the counter row before reading the issued numbers: an issue in
    // flight holds this row (via `allocate_number`'s upsert) until it commits,
    // so the guard waits for it and then sees its number — no stale check.
    counter::Entity::insert(counter::ActiveModel {
        series_id: Set(series_id),
        year: Set(year),
        last_number: Set(0),
    })
    .on_conflict(
        OnConflict::columns([counter::Column::SeriesId, counter::Column::Year])
            .do_nothing()
            .to_owned(),
    )
    .do_nothing()
    .exec(txn)
    .await?;
    counter::Entity::find_by_id((series_id, year))
        .lock_exclusive()
        .one(txn)
        .await?
        .context("counter row missing right after its upsert")?;
    let highest =
        crate::document::repo::query::highest_issued_seq(txn, space, doc_type, year).await?;
    if highest.is_some_and(|h| last_number < h) {
        return Err(AppError::field("lastNumber", "below_issued"));
    }
    counter::Entity::update_many()
        .col_expr(counter::Column::LastNumber, Expr::value(last_number))
        .filter(counter::Column::SeriesId.eq(series_id))
        .filter(counter::Column::Year.eq(year))
        .exec(txn)
        .await?;
    Ok(())
}

/// Allocate the next number of `doc_type`'s series for `year` (atomic
/// upsert; must run in the transaction that stores the numbered document).
pub async fn allocate_number(
    txn: &DatabaseTransaction,
    space: SpaceId,
    doc_type: DocType,
    year: i32,
) -> Result<(String, i32), AppError> {
    let series = load(txn, space, doc_type).await?;
    let pattern = Pattern::parse(&series.pattern)
        .with_context(|| format!("stored pattern of {} is invalid", doc_type.as_str()))?;
    let row = txn
        .query_one(Statement::from_sql_and_values(
            txn.get_database_backend(),
            "INSERT INTO number_series_counters (series_id, year, last_number) VALUES ($1, $2, 1) \
             ON CONFLICT (series_id, year) \
             DO UPDATE SET last_number = number_series_counters.last_number + 1 \
             RETURNING last_number",
            [series.id.into(), year.into()],
        ))
        .await?
        .context("counter upsert returned no row")?;
    let n: i32 = row.try_get("", "last_number")?;
    Ok((pattern.format(year, i64::from(n)), n))
}

async fn load<C: ConnectionTrait>(
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
