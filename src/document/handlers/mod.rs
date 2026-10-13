//! Axum handlers of `/api/documents`: extract, call the repos / the document
//! service, respond. Domain types live in `document::{model, snapshot, meta}`,
//! shared wire shapes in `document::wire`.

pub mod actions;
pub mod documents;
pub mod dto;
pub mod number_preview;
pub mod original;
pub mod payments;
pub mod received;
