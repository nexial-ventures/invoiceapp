//! `google_flows`, `google_handoffs`.

pub mod google_flow {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "google_flows")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub state_hash: String,
        /// The origin host's space (`None` = base host).
        pub space_id: Option<Uuid>,
        pub purpose: String,
        pub pkce_verifier: String,
        pub nonce: String,
        pub return_path: String,
        pub invite_hash: Option<String>,
        /// The invitation token sealed with the space id as associated data.
        pub invite_sealed: Option<Vec<u8>>,
        /// `link`: the signed-in user.
        pub user_id: Option<Uuid>,
        /// The per-flow name of the browser-binding cookie.
        pub browser_cookie: String,
        /// sha256 of that cookie's value.
        pub browser_hash: String,
        pub created_at: DateTimeWithTimeZone,
        pub expires_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod google_handoff {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "google_handoffs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub token_hash: String,
        pub space_id: Option<Uuid>,
        /// `link`: the signed-in user; `None` otherwise (resolved in finish).
        pub user_id: Option<Uuid>,
        pub purpose: String,
        pub return_path: String,
        pub invite_hash: Option<String>,
        pub invite_sealed: Option<Vec<u8>>,
        /// The verified ID token claims.
        pub google_sub: String,
        pub google_email: String,
        pub google_name: Option<String>,
        pub browser_cookie: String,
        pub browser_hash: String,
        pub created_at: DateTimeWithTimeZone,
        pub expires_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
