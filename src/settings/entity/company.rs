use sea_orm::entity::prelude::*;

/// The own company profile — one row per space, created with the space.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "company")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub space_id: Uuid,
    pub name: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub vat_payer: bool,
    pub street: String,
    pub city: String,
    pub zip: String,
    pub country: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub web: Option<String>,
    pub registration: Option<String>,
    pub default_due_days: i32,
    pub default_locale: String,
    /// `none` | `half` | `unit` (CHECK-constrained).
    pub default_rounding_cash: String,
    pub default_rounding_other: String,
    /// `bank_transfer` | `cash` | `card` | `other` (CHECK-constrained).
    pub default_payment_method: String,
    pub default_currency: String,
    pub default_header_note_cs: Option<String>,
    pub default_header_note_en: Option<String>,
    pub default_footer_note_cs: Option<String>,
    pub default_footer_note_en: Option<String>,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
