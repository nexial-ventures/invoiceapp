//! Accounting program exports ([api/accounting.md](../../docs/api/accounting.md),
//! [api/money.md](../../docs/api/money.md)): Settings → Accounting (the
//! codes per program × direction × document type) and the Pohoda and
//! Money S3 XML writers, formats of the accountant export
//! ([`crate::export`]).

pub mod doc;
pub mod entity;
pub mod export;
pub mod handlers;
pub mod money;
pub mod money_summary;
pub mod pohoda;
pub mod pohoda_summary;
pub mod repo;
pub mod settings;
