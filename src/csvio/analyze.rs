//! Upload → per-row outcome. Preview returns it; confirm recomputes it (the
//! server keeps nothing in between).

use std::collections::{HashMap, HashSet};

use anyhow::Context as _;
use sea_orm::DatabaseConnection;

use super::cell::{MISSING_FIELD, Row, RowError, text_of};
use super::columns::Columns;
use super::format as f;
use super::read::{self, FileError};
use super::row::{self, Ctx, Mapped};
use crate::document::doc_type::DocType;
use crate::document::line::Direction;
use crate::document::meta::category_kind;
use crate::error::AppError;
use crate::import::category;
use crate::import::check::{self, Checked};
use crate::import::model::{Code, Party, Plan};
use crate::settings::repo::company;
use crate::space::SpaceId;

pub const CATEGORY_CREATED: Code = "category_created";
pub const CATEGORY_INACTIVE: Code = "category_inactive";

/// What an error row still shows in the preview.
#[derive(Debug, Clone, Default)]
pub struct Raw {
    pub direction: Option<Direction>,
    pub doc_type: Option<&'static str>,
    pub number: Option<String>,
    /// Name and IČO.
    pub counterparty: Option<(String, Option<String>)>,
}

#[derive(Debug, Clone)]
pub struct Ready {
    pub mapped: Mapped,
    pub checked: Checked,
    /// `existing` | `new` (none: no category, or an inactive one).
    pub category_match: Option<&'static str>,
    /// The category warnings.
    pub warnings: Vec<Code>,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub line: u32,
    pub outcome: Result<Ready, (RowError, Raw)>,
}

type Parsed = (u32, Result<Mapped, (RowError, Raw)>);

fn raw(row: Row) -> Raw {
    let direction = row
        .text(f::DIRECTION)
        .and_then(|d| Direction::parse(&d.to_lowercase()));
    let number = if direction == Some(Direction::Received) {
        row.text(f::SUPPLIER_NUMBER)
    } else {
        row.text(f::NUMBER)
    };
    Raw {
        direction,
        doc_type: row
            .text(f::DOC_TYPE)
            .and_then(|t| DocType::parse_document(&t.to_lowercase()))
            .map(DocType::as_str),
        number,
        counterparty: row
            .text(f::COUNTERPARTY_NAME)
            .map(|n| (n, text_of(row.cell(f::COUNTERPARTY_ICO)))),
    }
}

/// Read and map every row: CPU-bound, run off the async workers. A
/// required column that is absent (a row misses it) is a file error.
fn parse(bytes: &[u8], ctx: &Ctx) -> Result<Vec<Parsed>, FileError> {
    let table = read::read(bytes)?;
    if table.rows.is_empty() {
        return Err(FileError::Empty);
    }
    let cols = Columns::new(&table.header)?;
    let parsed: Vec<Parsed> = table
        .rows
        .iter()
        .map(|r| {
            let row = Row {
                cols: &cols,
                cells: &r.cells,
            };
            (r.line, row::map(row, ctx).map_err(|e| (e, raw(row))))
        })
        .collect();
    let absent = parsed.iter().find_map(|(_, o)| match o {
        Err((e, _)) if e.code == MISSING_FIELD => e.field.as_ref().filter(|c| !cols.has(c)),
        _ => None,
    });
    match absent {
        Some(column) => Err(FileError::MissingColumn(column.clone())),
        None => Ok(parsed),
    }
}

pub fn file_error(e: FileError) -> AppError {
    match e.detail() {
        Some(d) => AppError::field_detail("file", e.reason(), d),
        None => AppError::field("file", e.reason()),
    }
}

async fn ctx(db: &DatabaseConnection, space: SpaceId) -> Result<Ctx, AppError> {
    let c = company::get(db, space).await?;
    Ok(Ctx {
        company: Party {
            name: c.name,
            ico: c.ico,
            dic: c.dic,
            street: c.street,
            city: c.city,
            zip: c.zip,
            country: c.country,
            registration: c.registration,
            email: c.email,
            phone: c.phone,
            web: c.web,
        },
        vat_payer: c.vat_payer,
        locale: c.default_locale,
    })
}

/// `only`: run the database lookups for these keys only (confirm: the
/// selected ones).
pub async fn analyze(
    db: &DatabaseConnection,
    space: SpaceId,
    bytes: Vec<u8>,
    only: Option<&HashSet<String>>,
) -> Result<Vec<Entry>, AppError> {
    let ctx = ctx(db, space).await?;
    let parsed = tokio::task::spawn_blocking(move || parse(&bytes, &ctx))
        .await
        .context("CSV parsing panicked")?
        .map_err(file_error)?;
    let keys: Vec<String> = parsed.iter().map(|p| format!("row:{}", p.0)).collect();
    let entries: Vec<(&str, Option<&Plan>)> = keys
        .iter()
        .zip(&parsed)
        .map(|(k, (_, o))| (k.as_str(), o.as_ref().ok().map(|m| &m.plan)))
        .collect();
    let checked = check::check(db, space, &entries, only).await?;
    drop(entries);
    let mut categories = Categories::new();
    let mut out = Vec::with_capacity(parsed.len());
    for ((key, (line, outcome)), checked) in keys.into_iter().zip(parsed).zip(checked) {
        let outcome = match (outcome, checked) {
            (Ok(mapped), Some(checked)) => {
                let mut ready = Ready {
                    mapped,
                    checked,
                    category_match: None,
                    warnings: Vec::new(),
                };
                if only.is_none_or(|o| o.contains(&key)) {
                    categorize(db, space, &mut categories, &mut ready).await?;
                }
                Ok(ready)
            }
            (Err(e), _) => Err(e),
            // `check` answers every planned entry.
            (Ok(_), None) => Err((
                RowError {
                    code: "internal",
                    field: None,
                },
                Raw::default(),
            )),
        };
        out.push(Entry { key, line, outcome });
    }
    Ok(out)
}

/// Category lookups of one request: (kind, lowercase name) → active.
type Categories = HashMap<(&'static str, String), Option<bool>>;

async fn categorize(
    db: &DatabaseConnection,
    space: SpaceId,
    cache: &mut Categories,
    r: &mut Ready,
) -> Result<(), AppError> {
    let Some(name) = &r.mapped.category else {
        return Ok(());
    };
    let kind = category_kind(r.mapped.plan.direction);
    let key = (kind, name.trim().to_lowercase());
    let active = match cache.get(&key) {
        Some(a) => *a,
        None => {
            let a = category::find(db, space, kind, name)
                .await?
                .map(|c| c.active);
            cache.insert(key, a);
            a
        }
    };
    match active {
        Some(true) => r.category_match = Some("existing"),
        Some(false) => r.warnings.push(CATEGORY_INACTIVE),
        None => {
            r.category_match = Some("new");
            r.warnings.push(CATEGORY_CREATED);
        }
    }
    Ok(())
}
