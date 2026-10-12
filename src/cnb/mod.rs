//! ČNB daily exchange rates: HTTP client, text-format parser, DB cache, route.

pub mod client;
pub mod entity;
pub mod handlers;
pub mod recent;
pub mod repo;

pub use client::{CnbClient, DEFAULT_CNB_URL};
