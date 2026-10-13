use sea_orm::entity::prelude::*;

/// Customer or supplier. `ico` is unique when present (partial unique index
/// `contacts_ico_key`). Invoices snapshot contacts, so edits never rewrite them.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "contacts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub space_id: Uuid,
    pub name: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub street: String,
    pub city: String,
    pub zip: String,
    pub country: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub note: Option<String>,
    pub default_due_days: Option<i32>,
    pub default_locale: Option<String>,
    pub default_currency: Option<String>,
    /// The `{C}` of client number patterns; unique per space, case-insensitive.
    pub number_code: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
