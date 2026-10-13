//! Phase 4c: TOTP two-factor authentication (`docs/api/mfa.md`): the user's
//! encrypted TOTP secret (active and pending), the replay guard, recovery
//! codes, pending second-step logins and the per-space policy.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
-- Secrets are AES-256-GCM sealed under INVOICEAPP__SECRET_KEY: nonce (12) || ciphertext || tag.
ALTER TABLE users
    ADD COLUMN totp_secret             bytea,
    ADD COLUMN totp_pending            bytea,
    ADD COLUMN totp_pending_expires_at timestamptz,
    -- The last accepted 30 s step: a code for this step or an earlier one is refused.
    ADD COLUMN totp_last_step          bigint;

CREATE TABLE recovery_codes (
    id         uuid        PRIMARY KEY,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- HMAC-SHA256 (hex) of the normalized code; a used code is deleted.
    code_hash  text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT recovery_codes_user_hash_key UNIQUE (user_id, code_hash)
);

-- A password-verified login waiting for its code (cookie invoice_mfa).
CREATE TABLE mfa_logins (
    id         uuid        PRIMARY KEY,
    -- sha256 (hex) of the cookie value.
    token_hash text        NOT NULL CONSTRAINT mfa_logins_token_hash_key UNIQUE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The host the login started on (NULL = the base host).
    space_id   uuid        REFERENCES spaces (id) ON DELETE CASCADE,
    user_agent text,
    failures   integer     NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);
CREATE INDEX mfa_logins_expires_at_idx ON mfa_logins (expires_at);

ALTER TABLE spaces ADD COLUMN require_mfa boolean NOT NULL DEFAULT false;
"#;

const DOWN: &str = r#"
ALTER TABLE spaces DROP COLUMN require_mfa;
DROP TABLE mfa_logins;
DROP TABLE recovery_codes;
ALTER TABLE users
    DROP COLUMN totp_secret,
    DROP COLUMN totp_pending,
    DROP COLUMN totp_pending_expires_at,
    DROP COLUMN totp_last_step;
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
