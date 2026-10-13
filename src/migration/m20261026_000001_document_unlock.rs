//! Unlocking an issued document back to a draft (`docs/api/documents.md`,
//! "Unlock"): who did it last, when and why.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
ALTER TABLE documents
    ADD COLUMN unlocked_at   timestamptz,
    ADD COLUMN unlocked_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN unlock_reason text,
    -- Saved (`PUT`) since the latest unlock: an unedited import re-issues as entered.
    ADD COLUMN edited_since_unlock boolean NOT NULL DEFAULT false;
CREATE INDEX documents_unlocked_by_idx ON documents (unlocked_by) WHERE unlocked_by IS NOT NULL;
"#;

const DOWN: &str = "DROP INDEX documents_unlocked_by_idx; \
                    ALTER TABLE documents DROP COLUMN edited_since_unlock, DROP COLUMN unlock_reason, \
                    DROP COLUMN unlocked_by, DROP COLUMN unlocked_at;";

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
