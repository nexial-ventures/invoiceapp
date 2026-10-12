//! The bulk import pipeline shared by the ISDOC and the CSV / XLSX import:
//! the planned document, database lookups (per entry and batched), the per-entry check of a
//! preview / confirm, the per-entry transaction, the multipart upload and
//! the wire types.

pub mod batch;
pub mod category;
pub mod check;
pub mod form;
pub mod lookup;
pub mod model;
pub mod store;
pub mod wire;
