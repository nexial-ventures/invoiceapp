//! Document numbers: allocation, read-only preview and reading a manual
//! number back (`docs/api/numbering.md`).
//!
//! A customer with a number code, on a series with a client pattern, gets
//! the next number of their client sequence; everyone else the global one.
//! Allocation takes the **next free number after the counter**: counter + 1,
//! skipping every number some document of the series already holds (native,
//! draft, imported — the unique number index's scope, case-insensitively),
//! and moves the counter to it. Only an allocation moves a counter.

use std::collections::HashSet;

use anyhow::Context as _;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder,
    Statement, Value,
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::number_series;
use crate::contact::entity::{contact, number_counter};
use crate::document::doc_type::DocType;
use crate::error::AppError;
use crate::settings::entity::number_series as series_entity;
use crate::settings::entity::number_series_counter as counter;
use crate::settings::pattern::{Matched, Pattern};
use crate::space::SpaceId;

/// Which counter numbers a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NumberSource {
    Client,
    Global,
}

/// The first sequence after `last` that `taken` does not hold.
pub fn next_free(last: i32, taken: impl Fn(i32) -> bool) -> Option<i32> {
    let mut n = last.checked_add(1)?;
    while taken(n) {
        n = n.checked_add(1)?;
    }
    Some(n)
}

/// A number taken from a counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allocated {
    pub number: String,
    pub year: i32,
    pub seq: i32,
    /// The customer code of a client sequence.
    pub code: Option<String>,
}

/// The counter and pattern a document of a series for a customer uses.
pub struct Scope {
    series: DocType,
    series_id: Uuid,
    pattern: Pattern,
    /// `(contact id, code)` of a client sequence.
    client: Option<(Uuid, String)>,
}

impl Scope {
    /// The client sequence needs both the customer's code and the series'
    /// client pattern; anything else numbers from the global series.
    pub fn of(
        row: &series_entity::Model,
        customer: Option<&contact::Model>,
    ) -> Result<Self, AppError> {
        let invalid = || format!("stored pattern of {} is invalid", row.doc_type.as_str());
        let coded = customer.and_then(|c| c.number_code.as_deref().map(|code| (c.id, code)));
        let (pattern, client) = match (coded, row.client_pattern.as_deref()) {
            (Some((id, code)), Some(p)) => (
                Pattern::parse_client(p).with_context(invalid)?,
                Some((id, code.to_string())),
            ),
            _ => (Pattern::parse(&row.pattern).with_context(invalid)?, None),
        };
        Ok(Self {
            series: row.doc_type,
            series_id: row.id,
            pattern,
            client,
        })
    }

    fn code(&self) -> Option<&str> {
        self.client.as_ref().map(|(_, c)| c.as_str())
    }

    pub fn source(&self) -> NumberSource {
        match self.client {
            Some(_) => NumberSource::Client,
            None => NumberSource::Global,
        }
    }

    /// The first free number after `last` in `year`.
    async fn next<C: ConnectionTrait>(
        &self,
        db: &C,
        space: SpaceId,
        year: i32,
        last: i32,
    ) -> Result<(String, i32), AppError> {
        let held = self.held(db, space, year).await?;
        let render = |n: i32| self.pattern.format(year, i64::from(n), self.code());
        let n = next_free(last, |n| held.contains(&render(n).to_lowercase()))
            .context("counter overflow")?;
        Ok((render(n), n))
    }

    /// The numbers (lowercase) of the series' documents that start like
    /// this scope's numbers of `year`.
    async fn held<C: ConnectionTrait>(
        &self,
        db: &C,
        space: SpaceId,
        year: i32,
    ) -> Result<HashSet<String>, AppError> {
        let prefix = self.pattern.prefix(year, self.code()).to_lowercase();
        let (direction, doc_type) = self.series.numbered();
        let rows = db
            .query_all(Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT number FROM documents WHERE space_id = $1 AND direction = $2 \
                 AND doc_type = $3 AND number IS NOT NULL AND left(lower(number), $4) = $5",
                [
                    space.into(),
                    direction.as_str().into(),
                    doc_type.as_str().into(),
                    i32::try_from(prefix.chars().count())
                        .context("prefix length")?
                        .into(),
                    prefix.clone().into(),
                ],
            ))
            .await?;
        rows.iter()
            .map(|r| Ok(r.try_get::<String>("", "number")?.to_lowercase()))
            .collect()
    }

    /// SQL for this scope's counter row: `(insert-if-missing, its key
    /// filter, the key values)`; the filter's parameters are `$1..`.
    fn counter_sql(&self, space: SpaceId, year: i32) -> CounterSql {
        match &self.client {
            Some((contact_id, _)) => CounterSql {
                insert: "INSERT INTO contact_number_counters \
                         (contact_id, series_id, year, space_id, last_number) \
                         VALUES ($1, $2, $3, $4, 0) ON CONFLICT DO NOTHING",
                table: "contact_number_counters",
                filter: "contact_id = $1 AND series_id = $2 AND year = $3",
                keys: vec![(*contact_id).into(), self.series_id.into(), year.into()],
                extra: Some(space.into()),
            },
            None => CounterSql {
                insert: "INSERT INTO number_series_counters (series_id, year, last_number) \
                         VALUES ($1, $2, 0) ON CONFLICT DO NOTHING",
                table: "number_series_counters",
                filter: "series_id = $1 AND year = $2",
                keys: vec![self.series_id.into(), year.into()],
                extra: None,
            },
        }
    }
}

struct CounterSql {
    insert: &'static str,
    table: &'static str,
    filter: &'static str,
    keys: Vec<Value>,
    /// The insert's value after the keys (the client counter's space).
    extra: Option<Value>,
}

/// Allocate the next free number of `series` for `year`; must run in the
/// transaction that stores the numbered document. The counter row is locked
/// first, so concurrent issues of one counter serialize and each sees the
/// numbers committed before it; the unique number index stays the backstop.
pub async fn allocate(
    txn: &DatabaseTransaction,
    space: SpaceId,
    series: DocType,
    year: i32,
    customer: Option<&contact::Model>,
) -> Result<Allocated, AppError> {
    let row = number_series::load(txn, space, series).await?;
    let s = Scope::of(&row, customer)?;
    let backend = txn.get_database_backend();
    let c = s.counter_sql(space, year);
    let stmt =
        |sql: String, values: Vec<Value>| Statement::from_sql_and_values(backend, sql, values);
    let mut insert_values = c.keys.clone();
    insert_values.extend(c.extra.clone());
    txn.execute(stmt(c.insert.to_string(), insert_values))
        .await?;
    let locked = txn
        .query_one(stmt(
            format!(
                "SELECT last_number FROM {} WHERE {} FOR UPDATE",
                c.table, c.filter
            ),
            c.keys.clone(),
        ))
        .await?
        .context("counter row missing right after its upsert")?;
    let last: i32 = locked.try_get("", "last_number")?;
    let (number, seq) = s.next(txn, space, year, last).await?;
    let mut values = c.keys.clone();
    values.push(seq.into());
    txn.execute(stmt(
        format!(
            "UPDATE {} SET last_number = ${} WHERE {}",
            c.table,
            values.len(),
            c.filter
        ),
        values,
    ))
    .await?;
    Ok(Allocated {
        number,
        year,
        seq,
        code: s.code().map(str::to_string),
    })
}

/// Allocate from the global series (received documents, tests).
pub async fn allocate_number(
    txn: &DatabaseTransaction,
    space: SpaceId,
    series: DocType,
    year: i32,
) -> Result<(String, i32), AppError> {
    let a = allocate(txn, space, series, year, None).await?;
    Ok((a.number, a.seq))
}

/// The number [`allocate`] would hand out now in `year`, nothing written.
pub async fn preview<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    series: DocType,
    year: i32,
    customer: Option<&contact::Model>,
) -> Result<(String, NumberSource), AppError> {
    let row = number_series::load(db, space, series).await?;
    let s = Scope::of(&row, customer)?;
    let last = match &s.client {
        Some((contact_id, _)) => {
            number_counter::Entity::find_by_id((*contact_id, s.series_id, year))
                .one(db)
                .await?
                .map(|c| c.last_number)
        }
        None => counter::Entity::find_by_id((s.series_id, year))
            .one(db)
            .await?
            .map(|c| c.last_number),
    };
    Ok((
        s.next(db, space, year, last.unwrap_or(0)).await?.0,
        s.source(),
    ))
}

/// The preview for an already loaded scope and its counter value.
pub async fn preview_in<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    scope: &Scope,
    year: i32,
    last: i32,
) -> Result<String, AppError> {
    Ok(scope.next(db, space, year, last).await?.0)
}

/// Read a manual `number` back (its year and sequence order the document
/// among the others): the customer's client pattern first, then the global
/// one, then the client pattern with any code. `None`: no pattern renders it.
pub async fn match_number<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    series: DocType,
    customer: Option<&contact::Model>,
    number: &str,
) -> Result<Option<Matched>, AppError> {
    let row = number_series::load(db, space, series).await?;
    let client = row
        .client_pattern
        .as_deref()
        .and_then(|p| Pattern::parse_client(p).ok());
    let code = customer.and_then(|c| c.number_code.as_deref());
    if let (Some(p), Some(code)) = (&client, code)
        && let Some(m) = p.matches(number, Some(code))
    {
        return Ok(Some(m));
    }
    if let Some(m) = Pattern::parse(&row.pattern)
        .ok()
        .and_then(|p| p.matches(number, None))
    {
        return Ok(Some(m));
    }
    Ok(client.and_then(|p| p.matches(number, None)))
}

/// A contact's client counters of `series`, newest year first.
pub async fn contact_counters<C: ConnectionTrait>(
    db: &C,
    contact_id: Uuid,
    series_id: Uuid,
) -> Result<Vec<number_counter::Model>, AppError> {
    Ok(number_counter::Entity::find()
        .filter(number_counter::Column::ContactId.eq(contact_id))
        .filter(number_counter::Column::SeriesId.eq(series_id))
        .order_by_desc(number_counter::Column::Year)
        .all(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(client_pattern: Option<&str>) -> series_entity::Model {
        series_entity::Model {
            id: Uuid::from_u128(1),
            space_id: Uuid::nil(),
            doc_type: DocType::Invoice,
            pattern: "{YYYY}{NNNN}".into(),
            client_pattern: client_pattern.map(str::to_string),
        }
    }

    fn customer(code: Option<&str>) -> contact::Model {
        let now = chrono::Utc::now().fixed_offset();
        contact::Model {
            id: Uuid::from_u128(2),
            space_id: Uuid::nil(),
            name: "Acme".into(),
            ico: None,
            dic: None,
            street: String::new(),
            city: String::new(),
            zip: String::new(),
            country: "CZ".into(),
            email: None,
            phone: None,
            note: None,
            default_due_days: None,
            default_locale: None,
            default_currency: None,
            number_code: code.map(str::to_string),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn client_sequence_needs_code_and_pattern() {
        let p = Some("{C}-{YYYY}{NNN}");
        let source = |row, c: Option<contact::Model>| {
            Scope::of(&series(row), c.as_ref()).expect("scope").source()
        };
        assert_eq!(
            source(p, Some(customer(Some("ACME")))),
            NumberSource::Client
        );
        assert_eq!(source(p, Some(customer(None))), NumberSource::Global);
        assert_eq!(source(p, None), NumberSource::Global);
        assert_eq!(
            source(None, Some(customer(Some("ACME")))),
            NumberSource::Global
        );
        let s = Scope::of(&series(p), Some(&customer(Some("ACME")))).expect("scope");
        assert_eq!(s.pattern.format(2026, 1, s.code()), "ACME-2026001");
    }

    #[test]
    fn next_free_skips_held_numbers() {
        assert_eq!(next_free(0, |_| false), Some(1));
        // Gaps below the counter stay; held numbers after it are skipped.
        assert_eq!(next_free(0, |n| [1, 2, 4].contains(&n)), Some(3));
        assert_eq!(next_free(3, |n| [1, 2, 4].contains(&n)), Some(5));
        assert_eq!(next_free(i32::MAX, |_| false), None);
        assert_eq!(next_free(i32::MAX - 1, |_| true), None);
    }
}
