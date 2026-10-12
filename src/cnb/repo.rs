//! Exchange-rate lookup through the `exchange_rates` cache.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ConnectionTrait, EntityTrait, Set};

use super::client::CnbClient;
use super::entity;
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rate {
    pub currency: String,
    /// The ČNB publication date of the list the rate comes from.
    pub date: NaiveDate,
    pub rate: Decimal,
}

/// CZK per 1 unit of `currency` for `date` (latest list on or before it).
/// Unknown currency → [`AppError::NotFound`]; ČNB failure →
/// [`AppError::CnbUnavailable`].
///
/// A fetched list (and the absence of `currency` from it) is cached only
/// once it can no longer change: when it was
/// published for exactly `date`, or `date` is already in the past. Today's
/// request before ČNB publishes (~14:30) returns yesterday's list, which must
/// not stick for the rest of the day; it is kept in memory for
/// [`super::recent::TTL`] instead.
pub async fn rate<C: ConnectionTrait>(
    db: &C,
    cnb: &CnbClient,
    currency: &str,
    date: NaiveDate,
    today: NaiveDate,
) -> Result<Rate, AppError> {
    if currency == "CZK" {
        return Ok(Rate {
            currency: currency.to_string(),
            date,
            rate: Decimal::ONE,
        });
    }
    if let Some(row) = entity::Entity::find_by_id((currency.to_string(), date))
        .one(db)
        .await?
    {
        let rate = row.rate.ok_or(AppError::NotFound)?;
        return Ok(Rate {
            currency: row.currency,
            date: row.published_date,
            rate: rate.normalize(),
        });
    }
    let (list, fetched) = match cnb.recent.get(date) {
        Some(list) => (list, false),
        None => (
            cnb.fetch(date).await.map_err(AppError::CnbUnavailable)?,
            true,
        ),
    };
    if !fetched {
        // A provisional list from memory: nothing to store.
    } else if list.published != date && date >= today {
        cnb.recent.put(date, list.clone());
    } else {
        let mut rows: Vec<_> = list
            .rates
            .iter()
            .map(|(code, rate)| (code.clone(), Some(*rate)))
            .collect();
        // Remember "not listed" too, so unknown currencies don't refetch.
        if list.get(currency).is_none() {
            rows.push((currency.to_string(), None));
        }
        let rows = rows.into_iter().map(|(code, rate)| entity::ActiveModel {
            currency: Set(code),
            requested_date: Set(date),
            published_date: Set(list.published),
            rate: Set(rate),
        });
        entity::Entity::insert_many(rows)
            .on_conflict(
                OnConflict::columns([entity::Column::Currency, entity::Column::RequestedDate])
                    .do_nothing()
                    .to_owned(),
            )
            .do_nothing()
            .exec(db)
            .await?;
    }
    let rate = list.get(currency).ok_or(AppError::NotFound)?;
    Ok(Rate {
        currency: currency.to_string(),
        date: list.published,
        rate,
    })
}
