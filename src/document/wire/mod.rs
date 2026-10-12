//! Wire shapes shared by the REST handlers and the MCP tools: request bodies
//! with their (pure) validation into `document::model`, and response DTOs.
//! No DB / repo access here.

pub mod compute_input;
pub mod dto;
pub mod fields;
pub mod imported;
pub mod input;
pub mod line_input;
pub mod line_out;
pub mod received_input;
