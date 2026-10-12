//! Settings: own company profile, bank accounts, VAT rates, number series,
//! categories, custom field definitions.

pub mod entity;
pub mod handlers;
pub mod pattern;
pub mod repo;

use axum::Router;
use axum::routing::{get, put};

use crate::app::AppState;
use handlers::{bank_accounts, categories, company, custom_fields, number_series, vat_rates};

/// Routes relative to `/api/settings`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/company",
            get(company::get_company).put(company::put_company),
        )
        .route(
            "/bank-accounts",
            get(bank_accounts::list).post(bank_accounts::create),
        )
        .route(
            "/bank-accounts/{id}",
            put(bank_accounts::update).delete(bank_accounts::delete),
        )
        .route("/vat-rates", get(vat_rates::list).post(vat_rates::create))
        .route(
            "/vat-rates/{id}",
            put(vat_rates::update).delete(vat_rates::delete),
        )
        .route(
            "/categories",
            get(categories::list).post(categories::create),
        )
        .route(
            "/categories/{id}",
            put(categories::update).delete(categories::delete),
        )
        .route(
            "/custom-fields",
            get(custom_fields::list).post(custom_fields::create),
        )
        .route(
            "/custom-fields/{id}",
            put(custom_fields::update).delete(custom_fields::delete),
        )
        .route(
            "/accounting",
            get(crate::accounting::handlers::get).put(crate::accounting::handlers::put),
        )
        .route("/number-series", get(number_series::list))
        .route("/number-series/{doc_type}", put(number_series::put_pattern))
        .route(
            "/number-series/{doc_type}/counters/{year}",
            put(number_series::put_counter),
        )
}
