//! Metadata lookups (category, custom field definitions) and the metadata
//! write allowed in every status.

use anyhow::Context as _;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, Set, TransactionTrait,
};
use uuid::Uuid;

use super::query;
use crate::document::custom_fields::{FieldDef, FieldType, Values};
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::document::meta::{Meta, MetaCtx};
use crate::error::AppError;
use crate::settings::entity::{category, custom_field};
use crate::space::SpaceId;

/// Active definitions applying to `direction`, by position.
pub async fn defs<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    direction: Direction,
) -> Result<Vec<FieldDef>, AppError> {
    custom_field::Entity::find()
        .filter(custom_field::Column::SpaceId.eq(space))
        .filter(custom_field::Column::Active.eq(true))
        .filter(custom_field::Column::AppliesTo.is_in([direction.as_str(), "both"]))
        .order_by_asc(custom_field::Column::Position)
        .order_by_asc(custom_field::Column::Key)
        .all(db)
        .await?
        .into_iter()
        .map(|d| {
            Ok(FieldDef {
                field_type: FieldType::parse(&d.field_type)
                    .with_context(|| format!("custom field {} has unknown type", d.key))?,
                key: d.key,
                options: d.options,
                required: d.required,
            })
        })
        .collect()
}

/// The stored custom field values of a document.
pub fn stored_fields(doc: &document::Model) -> Values {
    doc.custom_fields.as_object().cloned().unwrap_or_default()
}

/// `doc`: the saved document being changed, if any.
pub async fn load<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    direction: Direction,
    category_id: Option<Uuid>,
    doc: Option<&document::Model>,
) -> Result<MetaCtx, AppError> {
    let category = match category_id {
        Some(id) => {
            category::Entity::find_by_id(id)
                .filter(category::Column::SpaceId.eq(space))
                .one(db)
                .await?
        }
        None => None,
    };
    Ok(MetaCtx {
        category,
        defs: defs(db, space, direction).await?,
        stored_category: doc.and_then(|d| d.category_id),
        stored_fields: doc.map(stored_fields).unwrap_or_default(),
    })
}

/// Allowed in every status, both directions.
pub async fn set(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
    meta: Meta,
    internal_note: Option<String>,
) -> Result<(), AppError> {
    Ok(db
        .transaction(|txn| {
            Box::pin(async move {
                let doc = query::lock(txn, space, id).await?;
                let mut row: document::ActiveModel = doc.into();
                row.category_id = Set(meta.category_id);
                row.custom_fields = Set(serde_json::Value::Object(meta.custom_fields));
                row.internal_note = Set(internal_note);
                row.updated_at = Set(chrono::Utc::now().into());
                row.update(txn).await?;
                Ok(())
            })
        })
        .await?)
}
