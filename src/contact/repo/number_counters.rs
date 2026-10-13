//! A customer's client-sequence counters, one list per issued series with a
//! client pattern (`docs/api/numbering.md`).

use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use super::contacts;
use crate::contact::entity::{contact, number_counter};
use crate::document::doc_type::DocType;
use crate::error::AppError;
use crate::settings::entity::number_series;
use crate::settings::repo::{number_series as series_repo, numbering};
use crate::space::SpaceId;

/// One issued series as seen from a customer.
pub struct ContactSeries {
    pub doc_type: DocType,
    pub client_pattern: Option<String>,
    /// Newest year first.
    pub counters: Vec<number_counter::Model>,
    /// The customer's next number in `year`; only with a code and a pattern.
    pub next_number_preview: Option<String>,
}

/// Every issued series with a client pattern; `year` for the previews.
pub async fn list(
    db: &DatabaseConnection,
    space: SpaceId,
    contact_id: Uuid,
    year: i32,
) -> Result<Vec<ContactSeries>, AppError> {
    let contact = contacts::get(db, space, contact_id).await?;
    let rows = number_series::Entity::find()
        .filter(number_series::Column::SpaceId.eq(space))
        .filter(number_series::Column::ClientPattern.is_not_null())
        .all(db)
        .await?;
    let mut out = Vec::new();
    for doc_type in DocType::ALL.into_iter().filter(|d| d.is_document_type()) {
        if let Some(row) = rows.iter().find(|r| r.doc_type == doc_type) {
            out.push(item(db, space, &contact, row, year).await?);
        }
    }
    Ok(out)
}

/// Set a counter by hand (any value >= 0; allocation skips held numbers).
/// Only issued series → `docType: not_issued`.
pub async fn set(
    db: &DatabaseConnection,
    space: SpaceId,
    contact_id: Uuid,
    doc_type: DocType,
    year: i32,
    last_number: i32,
    preview_year: i32,
) -> Result<ContactSeries, AppError> {
    let contact = contacts::get(db, space, contact_id).await?;
    if !doc_type.is_document_type() {
        return Err(AppError::field("docType", "not_issued"));
    }
    let row = series_repo::load(db, space, doc_type).await?;
    number_counter::Entity::insert(number_counter::ActiveModel {
        space_id: Set(space.uuid()),
        contact_id: Set(contact.id),
        series_id: Set(row.id),
        year: Set(year),
        last_number: Set(last_number),
    })
    .on_conflict(
        OnConflict::columns([
            number_counter::Column::ContactId,
            number_counter::Column::SeriesId,
            number_counter::Column::Year,
        ])
        .update_column(number_counter::Column::LastNumber)
        .to_owned(),
    )
    .exec(db)
    .await?;
    item(db, space, &contact, &row, preview_year).await
}

async fn item<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    contact: &contact::Model,
    row: &number_series::Model,
    year: i32,
) -> Result<ContactSeries, AppError> {
    let counters = numbering::contact_counters(db, contact.id, row.id).await?;
    let scope = numbering::Scope::of(row, Some(contact))?;
    // Only the customer's own sequence has a preview here.
    let next_number_preview = match scope.source() {
        numbering::NumberSource::Client => {
            let last = counters
                .iter()
                .find(|c| c.year == year)
                .map_or(0, |c| c.last_number);
            Some(numbering::preview_in(db, space, &scope, year, last).await?)
        }
        numbering::NumberSource::Global => None,
    };
    Ok(ContactSeries {
        doc_type: row.doc_type,
        client_pattern: row.client_pattern.clone(),
        counters,
        next_number_preview,
    })
}
