//! Indexes on the referencing side of foreign keys that had none: deleting a
//! bank account, a user or a space (cascade / SET NULL) otherwise scans the
//! whole referencing table. Plus the received-document duplicate lookup by
//! supplier number (`import::lookup`).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
CREATE INDEX documents_bank_account_idx ON documents (bank_account_id)
    WHERE bank_account_id IS NOT NULL;
CREATE INDEX documents_received_supplier_number_idx ON documents (space_id, supplier_number)
    WHERE direction = 'received';
CREATE INDEX sessions_space_idx ON sessions (space_id) WHERE space_id IS NOT NULL;
CREATE INDEX mfa_logins_user_idx ON mfa_logins (user_id);
CREATE INDEX mfa_logins_space_idx ON mfa_logins (space_id) WHERE space_id IS NOT NULL;
CREATE INDEX google_flows_user_idx ON google_flows (user_id) WHERE user_id IS NOT NULL;
CREATE INDEX google_flows_space_idx ON google_flows (space_id) WHERE space_id IS NOT NULL;
CREATE INDEX google_handoffs_user_idx ON google_handoffs (user_id) WHERE user_id IS NOT NULL;
CREATE INDEX google_handoffs_space_idx ON google_handoffs (space_id) WHERE space_id IS NOT NULL;
CREATE INDEX api_tokens_user_idx ON api_tokens (user_id);
CREATE INDEX space_invites_invited_by_idx ON space_invites (invited_by);
CREATE INDEX design_versions_created_by_idx ON design_versions (created_by)
    WHERE created_by IS NOT NULL;
CREATE INDEX design_drafts_updated_by_idx ON design_drafts (updated_by)
    WHERE updated_by IS NOT NULL;
"#;

const DOWN: &str = r#"
DROP INDEX design_drafts_updated_by_idx;
DROP INDEX design_versions_created_by_idx;
DROP INDEX space_invites_invited_by_idx;
DROP INDEX api_tokens_user_idx;
DROP INDEX google_handoffs_space_idx;
DROP INDEX google_handoffs_user_idx;
DROP INDEX google_flows_space_idx;
DROP INDEX google_flows_user_idx;
DROP INDEX mfa_logins_space_idx;
DROP INDEX mfa_logins_user_idx;
DROP INDEX sessions_space_idx;
DROP INDEX documents_received_supplier_number_idx;
DROP INDEX documents_bank_account_idx;
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(UP).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(DOWN).await?;
        Ok(())
    }
}
