//! Per-client number sequences (`docs/api/numbering.md`): a customer's number
//! code, a client pattern per issued series, a counter per contact / series /
//! year, and what a document was numbered under.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
ALTER TABLE contacts ADD COLUMN number_code text;
CREATE UNIQUE INDEX contacts_number_code_key ON contacts (space_id, lower(number_code))
    WHERE number_code IS NOT NULL;

ALTER TABLE number_series ADD COLUMN client_pattern text;
ALTER TABLE number_series ADD CONSTRAINT number_series_space_id_id_key UNIQUE (space_id, id);

-- Space-scoped like the document FKs (m20261023_000001_tenant_fks): a counter
-- only ever pairs a contact and a series of the same space.
CREATE TABLE contact_number_counters (
    space_id    uuid    NOT NULL,
    contact_id  uuid    NOT NULL,
    series_id   uuid    NOT NULL,
    year        integer NOT NULL,
    last_number integer NOT NULL,
    PRIMARY KEY (contact_id, series_id, year),
    CONSTRAINT contact_number_counters_contact_id_fkey FOREIGN KEY (space_id, contact_id)
        REFERENCES contacts (space_id, id) ON DELETE CASCADE,
    CONSTRAINT contact_number_counters_series_id_fkey FOREIGN KEY (space_id, series_id)
        REFERENCES number_series (space_id, id) ON DELETE CASCADE
);
CREATE INDEX contact_number_counters_contact_idx ON contact_number_counters (contact_id, space_id);
CREATE INDEX contact_number_counters_series_idx ON contact_number_counters (series_id, space_id);

ALTER TABLE documents
    -- The customer code the number was allocated / matched under; NULL = global series.
    ADD COLUMN number_code text,
    -- The number an unlocked draft replaced when it was re-issued under another one.
    ADD COLUMN previous_number text,
    -- The number a native document had when it was last unlocked (cleared at re-issue).
    ADD COLUMN unlocked_number text;
UPDATE documents SET unlocked_number = number
    WHERE status = 'draft' AND unlocked_at IS NOT NULL AND NOT imported AND number IS NOT NULL;

DROP INDEX documents_series_idx;
CREATE INDEX documents_series_idx ON documents (space_id, doc_type, number_year, number_code)
    WHERE number_seq IS NOT NULL;
"#;

const DOWN: &str = r#"
DROP INDEX documents_series_idx;
CREATE INDEX documents_series_idx ON documents (space_id, doc_type, number_year)
    WHERE number_seq IS NOT NULL;
ALTER TABLE documents
    DROP COLUMN unlocked_number, DROP COLUMN previous_number, DROP COLUMN number_code;
DROP TABLE contact_number_counters;
ALTER TABLE number_series DROP CONSTRAINT number_series_space_id_id_key;
ALTER TABLE number_series DROP COLUMN client_pattern;
DROP INDEX contacts_number_code_key;
ALTER TABLE contacts DROP COLUMN number_code;
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
