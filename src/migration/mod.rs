//! Database migrations (sea-orm-migration).
//!
//! **Adding a migration:** create `src/migration/mYYYYMMDD_NNNNNN_<name>.rs`
//! implementing `MigrationTrait`, declare it with `mod` below and append
//! `Box::new(<module>::Migration)` to [`Migrator::migrations`]. Migrations are
//! append-only — never edit one that has shipped.

pub use sea_orm_migration::prelude::*;

mod m20261008_000001_settings;
mod m20261008_000002_contacts;
mod m20261009_000001_documents;
mod m20261010_000001_phase_1c;
mod m20261011_000001_pdf_archive;
mod m20261012_000001_received_import;
mod m20261013_000001_doc_types;
mod m20261014_000001_document_emails;
mod m20261015_000001_accounting_settings;
mod m20261016_000001_spaces;
mod m20261017_000001_space_invites;
mod m20261018_000001_mfa;
mod m20261019_000001_google;
mod m20261020_000001_defaults;
mod m20261021_000001_design;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261008_000001_settings::Migration),
            Box::new(m20261008_000002_contacts::Migration),
            Box::new(m20261009_000001_documents::Migration),
            Box::new(m20261010_000001_phase_1c::Migration),
            Box::new(m20261011_000001_pdf_archive::Migration),
            Box::new(m20261012_000001_received_import::Migration),
            Box::new(m20261013_000001_doc_types::Migration),
            Box::new(m20261014_000001_document_emails::Migration),
            Box::new(m20261015_000001_accounting_settings::Migration),
            Box::new(m20261016_000001_spaces::Migration),
            Box::new(m20261017_000001_space_invites::Migration),
            Box::new(m20261018_000001_mfa::Migration),
            Box::new(m20261019_000001_google::Migration),
            Box::new(m20261020_000001_defaults::Migration),
            Box::new(m20261021_000001_design::Migration),
        ]
    }
}
