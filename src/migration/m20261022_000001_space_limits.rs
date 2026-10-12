//! Hidden per-space limits (`docs/api/spaces.md`, "Limits"): one
//! `space_limits` row per space, changed only by an operator with SQL, and
//! what the storage quota counts — the archived PDF's size on `documents`
//! (the original's size is already there) and the design bytes, which only
//! a storage listing knows (`space_storage_usage`).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
CREATE TABLE space_limits (
    space_id            uuid        PRIMARY KEY REFERENCES spaces (id) ON DELETE CASCADE,
    -- 100 MiB; keep in sync with `space::limits::DEFAULT_STORAGE_QUOTA_BYTES`.
    storage_quota_bytes bigint      NOT NULL DEFAULT 104857600 CHECK (storage_quota_bytes >= 0),
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now()
);
INSERT INTO space_limits (space_id) SELECT id FROM spaces;

CREATE TABLE space_storage_usage (
    space_id     uuid        PRIMARY KEY REFERENCES spaces (id) ON DELETE CASCADE,
    -- Live + draft + kept versions, as of the last design listing or change.
    design_bytes bigint      NOT NULL DEFAULT 0 CHECK (design_bytes >= 0),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

-- Archives written before this migration stay NULL (counted as 0).
ALTER TABLE documents ADD COLUMN pdf_size bigint;
"#;

const DOWN: &str = "ALTER TABLE documents DROP COLUMN pdf_size; \
                    DROP TABLE space_storage_usage; DROP TABLE space_limits;";

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
