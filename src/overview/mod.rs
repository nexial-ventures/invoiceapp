//! The overview of a period ([api/overview.md](../../docs/api/overview.md)):
//! income / expense, VAT per rate, the monthly series and categories over the
//! accountant export's document set (`aggregate`, `period`), and today's
//! receivables / payables (`open`); `repo` reads both, `handlers` serves
//! `GET /api/overview`.

pub mod aggregate;
pub mod dto;
pub mod handlers;
pub mod open;
pub mod period;
pub mod repo;
