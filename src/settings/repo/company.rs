use anyhow::Context;
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set};

use crate::error::AppError;
use crate::settings::entity::company;
use crate::settings::handlers::company::Profile;
use crate::settings::handlers::company_defaults::DefaultsUpdate;
use crate::space::SpaceId;

pub async fn get<C: ConnectionTrait>(db: &C, space: SpaceId) -> Result<company::Model, AppError> {
    let row = company::Entity::find_by_id(space.uuid())
        .one(db)
        .await?
        .context("company row of the space missing (created with the space)")?;
    Ok(row)
}

/// Replace the profile with the (validated) input and set the given defaults.
pub async fn update(
    db: &DatabaseConnection,
    space: SpaceId,
    c: Profile,
    defaults: DefaultsUpdate,
) -> Result<company::Model, AppError> {
    let mut row = company::ActiveModel {
        space_id: Set(space.uuid()),
        name: Set(c.name),
        ico: Set(c.ico),
        dic: Set(c.dic),
        vat_payer: Set(c.vat_payer),
        street: Set(c.street),
        city: Set(c.city),
        zip: Set(c.zip),
        country: Set(c.country),
        email: Set(c.email),
        phone: Set(c.phone),
        web: Set(c.web),
        registration: Set(c.registration),
        default_due_days: Set(c.default_due_days),
        default_locale: Set(c.default_locale),
        updated_at: Set(chrono::Utc::now().into()),
        ..Default::default()
    };
    defaults.apply(&mut row);
    let row = row.update(db).await?;
    Ok(row)
}
