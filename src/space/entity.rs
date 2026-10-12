//! `spaces`, `space_members`, `space_limits` and `space_storage_usage`.

pub mod space {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "spaces")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub slug: String,
        pub name: String,
        pub created_at: DateTimeWithTimeZone,
        /// Logins on this host need TOTP (`docs/api/mfa.md`).
        pub require_mfa: bool,
        /// The creating user; the space cap counts these while they own them.
        pub created_by: Option<Uuid>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod member {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "space_members")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub space_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: Uuid,
        pub role: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Hidden per-space limits: no API changes them, an operator does with SQL.
pub mod limits {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "space_limits")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub space_id: Uuid,
        pub storage_quota_bytes: i64,
        pub email_space_per_hour: i32,
        pub email_space_per_day: i32,
        pub email_user_per_hour: i32,
        pub email_user_per_day: i32,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Stored bytes the database cannot sum from its rows (the design files).
pub mod usage {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "space_storage_usage")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub space_id: Uuid,
        pub design_bytes: i64,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
