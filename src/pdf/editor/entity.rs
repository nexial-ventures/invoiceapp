//! `design_drafts` and `design_versions`.

pub mod draft {
    use sea_orm::entity::prelude::*;

    /// The draft's deletions; the row is also the space's design lock.
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "design_drafts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub space_id: Uuid,
        pub deleted: Vec<String>,
        pub revision: i64,
        pub updated_at: DateTimeWithTimeZone,
        pub updated_by: Option<Uuid>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod version {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "design_versions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub space_id: Uuid,
        pub number: i32,
        pub kind: String,
        pub created_at: DateTimeWithTimeZone,
        pub created_by: Option<Uuid>,
        /// `[{ path, size, sha256 }]`.
        pub files: Json,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
