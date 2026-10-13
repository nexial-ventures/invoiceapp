use sea_orm::entity::prelude::*;

/// Last number of a customer's client sequence in one series and year
/// (`contact_number_counters`); a new year is a new row.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "contact_number_counters")]
pub struct Model {
    pub space_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub contact_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub series_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub year: i32,
    pub last_number: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
