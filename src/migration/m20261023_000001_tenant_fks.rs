//! Tenant isolation enforced by the database: a document may only point at a
//! contact, bank account, category or document of its own space. Each
//! single-column foreign key between space-scoped root tables becomes a
//! composite `(space_id, x_id) → t (space_id, id)` with the same `ON DELETE`
//! behaviour (`SET NULL (x_id)` nulls only the reference, never `space_id`;
//! needs PostgreSQL 15+).
//!
//! Not covered, because the referencing table has no `space_id` of its own
//! (it belongs to its parent row, and the repos only reach it through a
//! space-checked parent): `document_lines.advance_document_id`,
//! `documents.payment_id` → `payments`, and `catalog_group_members`
//! (group / item). Carrying the space there would mean a new column on
//! every child row; the repos resolve those references inside the space.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
ALTER TABLE contacts      ADD CONSTRAINT contacts_space_id_id_key      UNIQUE (space_id, id);
ALTER TABLE bank_accounts ADD CONSTRAINT bank_accounts_space_id_id_key UNIQUE (space_id, id);
ALTER TABLE categories    ADD CONSTRAINT categories_space_id_id_key    UNIQUE (space_id, id);
ALTER TABLE documents     ADD CONSTRAINT documents_space_id_id_key     UNIQUE (space_id, id);

ALTER TABLE documents
    DROP CONSTRAINT documents_contact_id_fkey,
    DROP CONSTRAINT documents_bank_account_id_fkey,
    DROP CONSTRAINT documents_category_id_fkey,
    DROP CONSTRAINT documents_related_document_id_fkey;
ALTER TABLE documents
    ADD CONSTRAINT documents_contact_id_fkey FOREIGN KEY (space_id, contact_id)
        REFERENCES contacts (space_id, id) ON DELETE SET NULL (contact_id),
    ADD CONSTRAINT documents_bank_account_id_fkey FOREIGN KEY (space_id, bank_account_id)
        REFERENCES bank_accounts (space_id, id) ON DELETE SET NULL (bank_account_id),
    -- No ON DELETE action: a category in use cannot be deleted (409, checked
    -- by the repo). Deferred, like the advance line FK below: deleting a
    -- space cascades to categories and documents in one statement, and an
    -- immediate check could run before the referencing document is gone.
    ADD CONSTRAINT documents_category_id_fkey FOREIGN KEY (space_id, category_id)
        REFERENCES categories (space_id, id) DEFERRABLE INITIALLY DEFERRED,
    ADD CONSTRAINT documents_related_document_id_fkey FOREIGN KEY (space_id, related_document_id)
        REFERENCES documents (space_id, id) ON DELETE SET NULL (related_document_id);

-- The space-delete cascade reaches an advance and the invoice deducting it
-- (via its lines) in one statement; with the composite self FK's SET NULL in
-- the same cascade, an immediate check fired before the line was deleted.
ALTER TABLE document_lines
    ALTER CONSTRAINT document_lines_advance_document_id_fkey DEFERRABLE INITIALLY DEFERRED;

-- Indexes for the composite FKs, the id column first (some queries filter by
-- it alone); they replace the single-column ones.
DROP INDEX documents_contact_idx;
DROP INDEX documents_category_idx;
DROP INDEX documents_related_idx;
CREATE INDEX documents_contact_space_idx ON documents (contact_id, space_id)
    WHERE contact_id IS NOT NULL;
CREATE INDEX documents_bank_account_space_idx ON documents (bank_account_id, space_id)
    WHERE bank_account_id IS NOT NULL;
CREATE INDEX documents_category_space_idx ON documents (category_id, space_id)
    WHERE category_id IS NOT NULL;
CREATE INDEX documents_related_space_idx ON documents (related_document_id, space_id)
    WHERE related_document_id IS NOT NULL;
"#;

const DOWN: &str = r#"
ALTER TABLE document_lines
    ALTER CONSTRAINT document_lines_advance_document_id_fkey NOT DEFERRABLE;
DROP INDEX documents_contact_space_idx;
DROP INDEX documents_bank_account_space_idx;
DROP INDEX documents_category_space_idx;
DROP INDEX documents_related_space_idx;
CREATE INDEX documents_contact_idx ON documents (contact_id);
CREATE INDEX documents_category_idx ON documents (category_id) WHERE category_id IS NOT NULL;
CREATE INDEX documents_related_idx ON documents (related_document_id)
    WHERE related_document_id IS NOT NULL;
ALTER TABLE documents
    DROP CONSTRAINT documents_contact_id_fkey,
    DROP CONSTRAINT documents_bank_account_id_fkey,
    DROP CONSTRAINT documents_category_id_fkey,
    DROP CONSTRAINT documents_related_document_id_fkey;
ALTER TABLE documents
    ADD CONSTRAINT documents_contact_id_fkey FOREIGN KEY (contact_id)
        REFERENCES contacts (id) ON DELETE SET NULL,
    ADD CONSTRAINT documents_bank_account_id_fkey FOREIGN KEY (bank_account_id)
        REFERENCES bank_accounts (id) ON DELETE SET NULL,
    ADD CONSTRAINT documents_category_id_fkey FOREIGN KEY (category_id)
        REFERENCES categories (id),
    ADD CONSTRAINT documents_related_document_id_fkey FOREIGN KEY (related_document_id)
        REFERENCES documents (id) ON DELETE SET NULL;
ALTER TABLE documents     DROP CONSTRAINT documents_space_id_id_key;
ALTER TABLE categories    DROP CONSTRAINT categories_space_id_id_key;
ALTER TABLE bank_accounts DROP CONSTRAINT bank_accounts_space_id_id_key;
ALTER TABLE contacts      DROP CONSTRAINT contacts_space_id_id_key;
"#;

/// Documents pointing into another space, per reference column.
const CROSS_SPACE: &str = r#"
SELECT
    (SELECT count(*) FROM documents d JOIN contacts t ON t.id = d.contact_id
        WHERE t.space_id <> d.space_id) AS contact,
    (SELECT count(*) FROM documents d JOIN bank_accounts t ON t.id = d.bank_account_id
        WHERE t.space_id <> d.space_id) AS bank_account,
    (SELECT count(*) FROM documents d JOIN categories t ON t.id = d.category_id
        WHERE t.space_id <> d.space_id) AS category,
    (SELECT count(*) FROM documents d JOIN documents t ON t.id = d.related_document_id
        WHERE t.space_id <> d.space_id) AS related_document
"#;

/// Refuse with a readable message (the counts) instead of a raw FK error.
async fn check_cross_space(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            CROSS_SPACE,
        ))
        .await?
        .ok_or_else(|| DbErr::Migration("cross-space check returned no row".into()))?;
    let mut found = Vec::new();
    for col in ["contact", "bank_account", "category", "related_document"] {
        let n: i64 = row.try_get("", col)?;
        if n > 0 {
            found.push(format!("{col}: {n}"));
        }
    }
    if found.is_empty() {
        return Ok(());
    }
    Err(DbErr::Migration(format!(
        "documents reference rows of another space ({}); fix them before migrating",
        found.join(", ")
    )))
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        check_cross_space(manager).await?;
        manager.get_connection().execute_unprepared(UP).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(DOWN).await?;
        Ok(())
    }
}
