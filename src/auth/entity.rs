//! `users`, `sessions`, `api_tokens`, `user_tokens`, `recovery_codes`,
//! `mfa_logins`.

pub mod user {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub email: String,
        pub display_name: String,
        /// `None`: an account created through Google that has no password yet.
        pub password_hash: Option<String>,
        pub email_verified_at: Option<DateTimeWithTimeZone>,
        pub disabled: bool,
        pub created_at: DateTimeWithTimeZone,
        /// The linked Google account's OpenID Connect subject.
        pub google_sub: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The TOTP columns of `users`, read only where a code is checked or
/// enrolment runs (the per-request auth query loads just
/// `totp_secret IS NOT NULL`). Never inserted through.
pub mod user_totp {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Sealed TOTP secret (AES-256-GCM); `Some` = TOTP enabled.
        pub totp_secret: Option<Vec<u8>>,
        /// Sealed secret of an unfinished setup, valid until
        /// `totp_pending_expires_at`.
        pub totp_pending: Option<Vec<u8>>,
        pub totp_pending_expires_at: Option<DateTimeWithTimeZone>,
        /// The last accepted TOTP step (replay guard).
        pub totp_last_step: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod session {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "sessions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub token_hash: String,
        pub user_id: Uuid,
        pub space_id: Option<Uuid>,
        pub created_at: DateTimeWithTimeZone,
        pub last_seen_at: DateTimeWithTimeZone,
        pub expires_at: DateTimeWithTimeZone,
        pub user_agent: Option<String>,
        /// `password` | `google` (`created_at` is the login time).
        pub login_method: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod api_token {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "api_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub space_id: Uuid,
        pub user_id: Uuid,
        pub name: String,
        pub prefix: String,
        pub token_hash: String,
        pub role: String,
        pub created_at: DateTimeWithTimeZone,
        /// Last valid day (UTC).
        pub expires_at: Option<Date>,
        pub last_used_at: Option<DateTimeWithTimeZone>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod user_token {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "user_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        /// `verify` | `reset`.
        pub kind: String,
        pub token_hash: String,
        pub created_at: DateTimeWithTimeZone,
        pub expires_at: DateTimeWithTimeZone,
        pub used_at: Option<DateTimeWithTimeZone>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod recovery_code {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "recovery_codes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        /// HMAC-SHA256 (hex) of the normalized code.
        pub code_hash: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod mfa_login {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "mfa_logins")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub token_hash: String,
        pub user_id: Uuid,
        /// The host's space (`None` = base host).
        pub space_id: Option<Uuid>,
        pub user_agent: Option<String>,
        pub failures: i32,
        pub created_at: DateTimeWithTimeZone,
        pub expires_at: DateTimeWithTimeZone,
        /// `password` | `google`: what the session will record.
        pub login_method: String,
        /// A Google invitation accept waiting for the code: sha256 of the
        /// invitation token (the membership is created with the session).
        pub invite_hash: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
