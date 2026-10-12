//! Database lookups that validation and defaults need.

use rust_decimal::Decimal;
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::{advance_sources, ddpp_correction, meta};
use crate::contact::entity::contact;
use crate::document::defaults;
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::document::model::{Context, DocumentData, Existing};
use crate::document::wire::input::DocumentInput;
use crate::error::AppError;
use crate::settings::entity::{bank_account, vat_rate};
use crate::settings::repo::company;
use crate::space::SpaceId;

/// What validation needs to know about a stored draft.
pub fn existing(doc: &document::Model) -> Result<Existing, AppError> {
    Ok(Existing {
        id: doc.id,
        doc_type: doc.doc_type,
        imported: doc.imported,
        related_document_id: doc.related_document_id,
        contact_id: doc.contact_id,
        vat_mode: doc.vat_mode,
        currency: doc.currency.clone(),
        locale: doc.locale.clone(),
        exchange_rate: doc.exchange_rate,
        number: doc.number.clone(),
        unlocked: doc.unlocked_at.is_some(),
    })
}

/// `doc`: the stored draft a `PUT` replaces.
pub async fn load(
    db: &DatabaseConnection,
    space: SpaceId,
    input: &DocumentInput,
    today: chrono::NaiveDate,
    doc: Option<&document::Model>,
) -> Result<Context, AppError> {
    let existing = doc.map(existing).transpose()?;
    let contact = match input.contact_id {
        Some(id) => contact(db, space, id).await?,
        None => None,
    };
    let related = match input.related_document_id {
        Some(id) => {
            document::Entity::find_by_id(id)
                .filter(document::Column::SpaceId.eq(space))
                .one(db)
                .await?
        }
        None => None,
    };
    Ok(Context {
        today,
        // POST fills defaults; PUT (an existing draft) replaces every field.
        apply_defaults: existing.is_none(),
        company: company::get(db, space).await?,
        contact,
        default_vat_rate: default_vat_rate(db, space).await?,
        advances: advance_sources::load(db, space, &input.advance_ids()).await?,
        related,
        meta: meta::load(db, space, Direction::Issued, input.category_id, doc).await?,
        exact: ddpp_correction::basis_for(db, existing.as_ref()).await?,
        existing,
    })
}

/// A contact of `space` (another space's id: `None`, as if unknown).
pub async fn contact<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    id: Uuid,
) -> Result<Option<contact::Model>, AppError> {
    Ok(contact::Entity::find_by_id(id)
        .filter(contact::Column::SpaceId.eq(space))
        .one(db)
        .await?)
}

pub async fn default_vat_rate<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
) -> Result<Option<Decimal>, AppError> {
    Ok(vat_rate::Entity::find()
        .filter(vat_rate::Column::SpaceId.eq(space))
        .filter(vat_rate::Column::IsDefault.eq(true))
        .one(db)
        .await?
        .map(|r| r.rate.normalize()))
}

pub async fn bank_account<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    id: Uuid,
) -> Result<Option<bank_account::Model>, AppError> {
    Ok(bank_account::Entity::find_by_id(id)
        .filter(bank_account::Column::SpaceId.eq(space))
        .one(db)
        .await?)
}

/// The named bank account must exist and match the currency; with defaults
/// on, a missing one becomes the currency's default account (if any).
pub async fn resolve_bank(
    db: &DatabaseConnection,
    space: SpaceId,
    data: &mut DocumentData,
    apply_defaults: bool,
) -> Result<(), AppError> {
    match data.bank_account_id {
        Some(id) => match bank_account(db, space, id).await? {
            Some(acc) if acc.currency == data.currency => Ok(()),
            _ => Err(AppError::field("bankAccountId", "invalid")),
        },
        None if apply_defaults => {
            data.bank_account_id = bank_account::Entity::find()
                .filter(bank_account::Column::SpaceId.eq(space))
                .filter(bank_account::Column::Currency.eq(data.currency.as_str()))
                .filter(bank_account::Column::IsDefault.eq(true))
                .one(db)
                .await?
                .map(|a| a.id);
            Ok(())
        }
        None => Ok(()),
    }
}

/// `from` + the (contact ?? company) default due days, for server-created drafts.
pub async fn due_date(
    db: &DatabaseConnection,
    space: SpaceId,
    contact_id: Option<Uuid>,
    from: chrono::NaiveDate,
) -> Result<chrono::NaiveDate, AppError> {
    let contact = match contact_id {
        Some(id) => contact(db, space, id).await?,
        None => None,
    };
    let company = company::get(db, space).await?;
    Ok(defaults::due_date(contact.as_ref(), &company, from))
}
