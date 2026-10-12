use sea_orm::entity::prelude::*;

/// One send attempt of a document (append-only).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "document_emails")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub document_id: Uuid,
    pub created_at: DateTimeWithTimeZone,
    pub to_addrs: Vec<String>,
    pub cc_addrs: Vec<String>,
    pub bcc_addrs: Vec<String>,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<String>,
    pub ok: bool,
    pub error: Option<String>,
    pub message_id: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// One e-mail sent on behalf of a space, counted by the e-mail limits
/// (`email::limits`); purged after 2 days.
pub mod email_send {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "email_sends")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        /// `NULL` once the space is deleted (the user's count stays).
        pub space_id: Option<Uuid>,
        pub user_id: Uuid,
        /// `document` | `test` | `invite`.
        pub kind: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
