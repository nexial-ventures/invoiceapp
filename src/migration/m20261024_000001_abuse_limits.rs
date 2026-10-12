//! Limits for public registration (`docs/api/spaces.md`, "Limits"): the
//! outbound e-mail limits per space and per sending user (new
//! `space_limits` columns, the `email_sends` log they count) and the creator
//! of a space, which the per-user space cap counts.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
-- Keep in sync with `space::limits::Limits::default()`.
ALTER TABLE space_limits
    ADD COLUMN email_space_per_hour integer NOT NULL DEFAULT 20  CHECK (email_space_per_hour >= 0),
    ADD COLUMN email_space_per_day  integer NOT NULL DEFAULT 100 CHECK (email_space_per_day >= 0),
    ADD COLUMN email_user_per_hour  integer NOT NULL DEFAULT 20  CHECK (email_user_per_hour >= 0),
    ADD COLUMN email_user_per_day   integer NOT NULL DEFAULT 100 CHECK (email_user_per_day >= 0);

-- One row per e-mail sent on behalf of a space. `space_id` survives the
-- space's deletion as NULL, so deleting and recreating a space does not
-- reset its user's count. Purged after 2 days (`cleanup`).
CREATE TABLE email_sends (
    id         bigserial   PRIMARY KEY,
    space_id   uuid        REFERENCES spaces (id) ON DELETE SET NULL,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind       text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX email_sends_space_idx ON email_sends (space_id, created_at) WHERE space_id IS NOT NULL;
CREATE INDEX email_sends_user_idx ON email_sends (user_id, created_at);
CREATE INDEX email_sends_created_idx ON email_sends (created_at);

-- Who created the space (the space cap counts these while they own them).
-- Existing spaces: their earliest owner.
ALTER TABLE spaces ADD COLUMN created_by uuid REFERENCES users (id) ON DELETE SET NULL;
UPDATE spaces s SET created_by = (
    SELECT m.user_id FROM space_members m
     WHERE m.space_id = s.id AND m.role = 'owner'
     ORDER BY m.created_at, m.user_id LIMIT 1);
CREATE INDEX spaces_created_by_idx ON spaces (created_by) WHERE created_by IS NOT NULL;
"#;

const DOWN: &str = "ALTER TABLE spaces DROP COLUMN created_by; DROP TABLE email_sends; \
                    ALTER TABLE space_limits DROP COLUMN email_space_per_hour, \
                    DROP COLUMN email_space_per_day, DROP COLUMN email_user_per_hour, \
                    DROP COLUMN email_user_per_day;";

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
