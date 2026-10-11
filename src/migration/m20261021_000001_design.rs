//! Phase 4f: the per-space PDF design draft and its activation history
//! (`docs/api/design.md`). The files live in storage; these tables hold the
//! draft deletions (doubling as the per-space design lock row) and one row
//! per activation / revert / reset.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
CREATE TABLE design_drafts (
    space_id   uuid        PRIMARY KEY REFERENCES spaces (id) ON DELETE CASCADE,
    -- Live custom files the activation removes (the default shows through).
    deleted    text[]      NOT NULL DEFAULT '{}',
    -- Bumped by every draft change: an activation checked outside the lock
    -- applies only the draft revision it rendered.
    revision   bigint      NOT NULL DEFAULT 0,
    updated_at timestamptz NOT NULL DEFAULT now(),
    updated_by uuid        REFERENCES users (id) ON DELETE SET NULL
);

CREATE TABLE design_versions (
    id         uuid        PRIMARY KEY,
    space_id   uuid        NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    number     integer     NOT NULL,
    kind       text        NOT NULL CHECK (kind IN ('activation', 'revert', 'default')),
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid        REFERENCES users (id) ON DELETE SET NULL,
    -- [{ path, size, sha256 }] of the live set right after this version.
    files      jsonb       NOT NULL DEFAULT '[]',
    CONSTRAINT design_versions_space_number_key UNIQUE (space_id, number)
);
"#;

const DOWN: &str = "DROP TABLE design_versions; DROP TABLE design_drafts;";

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
