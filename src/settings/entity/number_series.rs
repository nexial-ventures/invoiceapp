use sea_orm::entity::prelude::*;

use crate::document::doc_type::DocType;

/// Number pattern per series (`doc_type`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "number_series")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub space_id: Uuid,
    pub doc_type: DocType,
    pub pattern: String,
    /// Issued series only: the pattern of customers with a number code.
    pub client_pattern: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
