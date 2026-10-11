//! Phase 4e: document defaults per space and rounding modes
//! (`docs/api/defaults.md`): `documents.round_total` becomes `rounding_mode`,
//! the company profile gains the defaults of new documents.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
ALTER TABLE documents
    ADD COLUMN rounding_mode text NOT NULL DEFAULT 'none'
        CONSTRAINT documents_rounding_mode_check CHECK (rounding_mode IN ('none', 'half', 'unit'));

-- The flag maps directly; an explicit `true` stays `unit` (e.g. an imported draft).
UPDATE documents SET rounding_mode = CASE WHEN round_total THEN 'unit' ELSE 'none' END;

-- Received documents and imported ones without the flag keep their rounding amount; the mode
-- is derived from it.
UPDATE documents SET rounding_mode = CASE
        WHEN rounding = 0 THEN 'none'
        WHEN mod(total + rounding, 1) = 0 THEN 'unit'
        WHEN mod((total + rounding) * 2, 1) = 0 THEN 'half'
        ELSE 'none'
    END
WHERE direction = 'received' OR (imported AND NOT round_total);

ALTER TABLE documents DROP COLUMN round_total;

ALTER TABLE company
    ADD COLUMN default_rounding_cash text NOT NULL DEFAULT 'unit'
        CONSTRAINT company_default_rounding_cash_check
            CHECK (default_rounding_cash IN ('none', 'half', 'unit')),
    ADD COLUMN default_rounding_other text NOT NULL DEFAULT 'none'
        CONSTRAINT company_default_rounding_other_check
            CHECK (default_rounding_other IN ('none', 'half', 'unit')),
    ADD COLUMN default_payment_method text NOT NULL DEFAULT 'bank_transfer'
        CONSTRAINT company_default_payment_method_check
            CHECK (default_payment_method IN ('bank_transfer', 'cash', 'card', 'other')),
    ADD COLUMN default_currency text NOT NULL DEFAULT 'CZK',
    ADD COLUMN default_header_note_cs text,
    ADD COLUMN default_header_note_en text,
    ADD COLUMN default_footer_note_cs text,
    ADD COLUMN default_footer_note_en text;
"#;

// The 1b flag existed only for issued native CZK documents (`half` has no
// equivalent): everything else goes back as `false`; the amounts are kept.
const DOWN: &str = r#"
ALTER TABLE company
    DROP COLUMN default_rounding_cash,
    DROP COLUMN default_rounding_other,
    DROP COLUMN default_payment_method,
    DROP COLUMN default_currency,
    DROP COLUMN default_header_note_cs,
    DROP COLUMN default_header_note_en,
    DROP COLUMN default_footer_note_cs,
    DROP COLUMN default_footer_note_en;
ALTER TABLE documents ADD COLUMN round_total boolean NOT NULL DEFAULT false;
UPDATE documents SET round_total = (
    rounding_mode = 'unit' AND currency = 'CZK' AND direction = 'issued' AND NOT imported
);
ALTER TABLE documents DROP COLUMN rounding_mode;
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
