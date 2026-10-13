//! Phase 4d: Sign in with Google (`docs/api/google.md`): accounts without a
//! password, the linked Google `sub`, the login method of sessions and
//! pending TOTP logins, the OIDC flow rows and the cross-host handoffs.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
ALTER TABLE users
    ALTER COLUMN password_hash DROP NOT NULL,
    -- The OpenID Connect subject of the linked Google account.
    ADD COLUMN google_sub text CONSTRAINT users_google_sub_key UNIQUE;

-- How the session was signed in; its created_at is the login time.
ALTER TABLE sessions
    ADD COLUMN login_method text NOT NULL DEFAULT 'password'
        CONSTRAINT sessions_login_method_check CHECK (login_method IN ('password', 'google'));

ALTER TABLE mfa_logins
    ADD COLUMN login_method text NOT NULL DEFAULT 'password'
        CONSTRAINT mfa_logins_login_method_check CHECK (login_method IN ('password', 'google')),
    -- A Google invitation accept waiting for the code: sha256 (hex) of the invitation token.
    ADD COLUMN invite_hash text;

-- An authorization request sent to Google (10 min, single use).
CREATE TABLE google_flows (
    id            uuid        PRIMARY KEY,
    -- sha256 (hex) of the OAuth state.
    state_hash    text        NOT NULL CONSTRAINT google_flows_state_hash_key UNIQUE,
    -- The host the flow started on (NULL = the base host).
    space_id      uuid        REFERENCES spaces (id) ON DELETE CASCADE,
    purpose       text        NOT NULL
        CONSTRAINT google_flows_purpose_check CHECK (purpose IN ('login', 'link', 'invite')),
    pkce_verifier text        NOT NULL,
    nonce         text        NOT NULL,
    return_path   text        NOT NULL,
    -- invite: sha256 (hex) of the invitation token, and the token sealed under
    -- INVOICEAPP__SECRET_KEY (associated data = the space id) for the error redirect.
    invite_hash   text,
    invite_sealed bytea,
    -- link: the signed-in user.
    user_id       uuid        REFERENCES users (id) ON DELETE CASCADE,
    -- The browser binding: start sets cookie `browser_cookie` (a per-flow name) on the origin
    -- host; sha256 (hex) of its value. Only the browser that started the flow can finish it.
    browser_cookie text       NOT NULL,
    browser_hash  text        NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL
);
CREATE INDEX google_flows_expires_at_idx ON google_flows (expires_at);

-- The callback's result, carried to the origin host (60 s, single use).
CREATE TABLE google_handoffs (
    id            uuid        PRIMARY KEY,
    token_hash    text        NOT NULL CONSTRAINT google_handoffs_token_hash_key UNIQUE,
    space_id      uuid        REFERENCES spaces (id) ON DELETE CASCADE,
    -- link: the signed-in user (login / invite resolve the user only in finish).
    user_id       uuid        REFERENCES users (id) ON DELETE CASCADE,
    purpose       text        NOT NULL
        CONSTRAINT google_handoffs_purpose_check CHECK (purpose IN ('login', 'link', 'invite')),
    return_path   text        NOT NULL,
    invite_hash   text,
    invite_sealed bytea,
    -- The verified ID token claims.
    google_sub    text        NOT NULL,
    google_email  text        NOT NULL,
    google_name   text,
    browser_cookie text       NOT NULL,
    browser_hash  text        NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL
);
CREATE INDEX google_handoffs_expires_at_idx ON google_handoffs (expires_at);

-- The single-use grant of POST /api/account/google/link (password + step-up checked).
ALTER TABLE user_tokens DROP CONSTRAINT user_tokens_kind_check;
ALTER TABLE user_tokens ADD CONSTRAINT user_tokens_kind_check
    CHECK (kind IN ('verify', 'reset', 'google_link'));
"#;

const DOWN: &str = r#"
DELETE FROM user_tokens WHERE kind = 'google_link';
ALTER TABLE user_tokens DROP CONSTRAINT user_tokens_kind_check;
ALTER TABLE user_tokens ADD CONSTRAINT user_tokens_kind_check CHECK (kind IN ('verify', 'reset'));
DROP TABLE google_handoffs;
DROP TABLE google_flows;
ALTER TABLE mfa_logins DROP COLUMN login_method, DROP COLUMN invite_hash;
ALTER TABLE sessions DROP COLUMN login_method;
DELETE FROM users WHERE password_hash IS NULL;
ALTER TABLE users
    DROP COLUMN google_sub,
    ALTER COLUMN password_hash SET NOT NULL;
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
