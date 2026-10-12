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
    -- No ON DELETE action: a category in use cannot be deleted (409).
    ADD CONSTRAINT documents_category_id_fkey FOREIGN KEY (space_id, category_id)
        REFERENCES categories (space_id, id),
    ADD CONSTRAINT documents_related_document_id_fkey FOREIGN KEY (space_id, related_document_id)
        REFERENCES documents (space_id, id) ON DELETE SET NULL (related_document_id);
"#;

const DOWN: &str = r#"
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
