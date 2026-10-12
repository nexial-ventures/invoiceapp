//! OpenAPI document served at `/api/openapi.json`.

use axum::Json;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};

#[derive(OpenApi)]
#[openapi(
    info(title = "Invoice API"),
    paths(
        crate::health::health,
        crate::auth::host::context,
        crate::auth::handlers::register::register,
        crate::auth::handlers::register::verify,
        crate::auth::handlers::register::resend,
        crate::auth::handlers::login::login,
        crate::auth::mfa::login::login_mfa,
        crate::auth::google::handlers::start,
        crate::auth::google::handlers::callback,
        crate::auth::google::finish::finish,
        crate::auth::google::account::link,
        crate::auth::google::account::unlink,
        crate::auth::mfa::handlers::status,
        crate::auth::mfa::handlers::setup,
        crate::auth::mfa::handlers::enable,
        crate::auth::mfa::handlers::disable,
        crate::auth::mfa::handlers::regenerate,
        crate::auth::handlers::login::logout,
        crate::auth::handlers::login::me,
        crate::auth::handlers::login::revoke_others,
        crate::auth::handlers::password::change,
        crate::auth::handlers::password::request_reset,
        crate::auth::handlers::password::confirm_reset,
        crate::space::handlers::list,
        crate::space::handlers::create,
        crate::space::handlers::get,
        crate::space::handlers::update,
        crate::space::handlers::delete,
        crate::auth::tokens::list,
        crate::auth::tokens::create,
        crate::auth::tokens::revoke,
        crate::members::handlers::list,
        crate::members::handlers::update,
        crate::members::handlers::remove,
        crate::members::handlers::leave,
        crate::members::invite_handlers::list,
        crate::members::invite_handlers::create,
        crate::members::invite_handlers::resend,
        crate::members::invite_handlers::revoke,
        crate::members::accept::show,
        crate::members::accept::accept,
        crate::settings::handlers::company::get_company,
        crate::settings::handlers::company::put_company,
        crate::settings::handlers::bank_accounts::list,
        crate::settings::handlers::bank_accounts::create,
        crate::settings::handlers::bank_accounts::update,
        crate::settings::handlers::bank_accounts::delete,
        crate::settings::handlers::vat_rates::list,
        crate::settings::handlers::vat_rates::create,
        crate::settings::handlers::vat_rates::update,
        crate::settings::handlers::vat_rates::delete,
        crate::accounting::handlers::get,
        crate::accounting::handlers::put,
        crate::settings::handlers::number_series::list,
        crate::settings::handlers::number_series::put_pattern,
        crate::settings::handlers::number_series::put_counter,
        crate::contact::handlers::contacts::list,
        crate::contact::handlers::contacts::create,
        crate::contact::handlers::contacts::get,
        crate::contact::handlers::contacts::update,
        crate::contact::handlers::contacts::delete,
        crate::ares::handlers::lookup,
        crate::document::handlers::documents::list,
        crate::document::handlers::documents::create,
        crate::document::handlers::documents::get,
        crate::document::handlers::documents::update,
        crate::document::handlers::documents::delete,
        crate::document::handlers::documents::compute,
        crate::document::handlers::actions::issue,
        crate::document::handlers::actions::cancel,
        crate::document::handlers::actions::mark_sent,
        crate::document::handlers::actions::internal_note,
        crate::document::handlers::actions::metadata,
        crate::document::handlers::original::put,
        crate::document::handlers::original::delete,
        crate::document::handlers::actions::settle,
        crate::document::handlers::actions::credit_note,
        crate::document::handlers::actions::debit_note,
        crate::document::handlers::payments::list,
        crate::document::handlers::payments::create,
        crate::document::handlers::payments::delete,
        crate::cnb::handlers::get_rate,
        crate::pdf::handlers::document_pdf,
        crate::pdf::handlers::preview,
        crate::pdf::editor::handlers::design,
        crate::pdf::editor::handlers::file,
        crate::pdf::editor::handlers::put_file,
        crate::pdf::editor::handlers::delete_file,
        crate::pdf::editor::handlers::discard,
        crate::pdf::editor::preview::preview,
        crate::pdf::editor::activate::activate,
        crate::pdf::editor::activate::versions,
        crate::pdf::editor::activate::restore,
        crate::pdf::editor::activate::reset,
        crate::catalog::handlers::items::list,
        crate::catalog::handlers::items::create,
        crate::catalog::handlers::items::get,
        crate::catalog::handlers::items::update,
        crate::catalog::handlers::items::delete,
        crate::catalog::handlers::groups::list,
        crate::catalog::handlers::groups::create,
        crate::catalog::handlers::groups::get,
        crate::catalog::handlers::groups::update,
        crate::catalog::handlers::groups::delete,
        crate::settings::handlers::categories::list,
        crate::settings::handlers::categories::create,
        crate::settings::handlers::categories::update,
        crate::settings::handlers::categories::delete,
        crate::settings::handlers::custom_fields::list,
        crate::settings::handlers::custom_fields::create,
        crate::settings::handlers::custom_fields::update,
        crate::settings::handlers::custom_fields::delete,
        crate::isdoc::import::preview,
        crate::isdoc::import::confirm,
        crate::csvio::handlers::preview,
        crate::csvio::handlers::confirm,
        crate::csvio::handlers::sample,
        crate::csvio::export::list_csv,
        crate::csvio::export::accountant,
        crate::isdoc::export::document_isdoc,
        crate::isdoc::export::bulk,
        crate::email::handlers::settings::status,
        crate::email::handlers::settings::test,
        crate::email::handlers::settings::list_templates,
        crate::email::handlers::settings::put_template,
        crate::email::handlers::settings::delete_template,
        crate::email::handlers::settings::preview,
        crate::email::handlers::document::prefill,
        crate::email::handlers::document::send,
        crate::email::handlers::document::history,
        crate::mcp::openapi_mcp,
    ),
    components(schemas(
        crate::health::HealthResponse,
        crate::error::ErrorBody,
        crate::document::handlers::received_input::ReceivedInput,
        crate::document::handlers::received_input::RecapInput,
        crate::isdoc::import::OptionsInput,
        crate::csvio::handlers::CsvOptionsInput,
    )),
    modifiers(&SecuritySchemes)
)]
pub struct ApiDoc;

/// `cookie`: the `invoice_session` cookie of a browser login; `bearer`: a
/// personal API token (`inv_…`). Both only on the space's own host.
struct SecuritySchemes;

impl Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
        components.add_security_scheme(
            "cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(
                crate::auth::session::COOKIE,
            ))),
        );
    }
}

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_the_routes() {
        let doc = ApiDoc::openapi();
        assert!(doc.paths.paths.contains_key("/api/health"));
        for path in [
            "/api/context",
            "/api/auth/register",
            "/api/auth/verify",
            "/api/auth/verify/resend",
            "/api/auth/login",
            "/api/auth/logout",
            "/api/auth/me",
            "/api/auth/sessions/revoke-others",
            "/api/account/password",
            "/api/auth/login/mfa",
            "/api/account/mfa",
            "/api/account/mfa/setup",
            "/api/account/mfa/enable",
            "/api/account/mfa/disable",
            "/api/account/mfa/recovery-codes",
            "/api/auth/password-reset",
            "/api/auth/password-reset/confirm",
            "/api/spaces",
            "/api/space",
            "/api/tokens",
            "/api/tokens/{id}",
            "/api/members",
            "/api/members/{userId}",
            "/api/space/leave",
            "/api/invites",
            "/api/invites/{id}",
            "/api/invites/{id}/resend",
            "/api/invites/accept",
            "/api/settings/company",
            "/api/settings/bank-accounts/{id}",
            "/api/settings/vat-rates",
            "/api/settings/accounting",
            "/api/settings/number-series/{docType}/counters/{year}",
            "/api/contacts",
            "/api/contacts/{id}",
            "/api/ares/{ico}",
            "/api/documents",
            "/api/documents/compute",
            "/api/documents/{id}",
            "/api/documents/{id}/issue",
            "/api/documents/{id}/cancel",
            "/api/documents/{id}/mark-sent",
            "/api/documents/{id}/internal-note",
            "/api/documents/{id}/metadata",
            "/api/documents/{id}/original",
            "/api/settings/categories",
            "/api/settings/categories/{id}",
            "/api/settings/custom-fields",
            "/api/settings/custom-fields/{id}",
            "/api/documents/{id}/payments",
            "/api/documents/{id}/payments/{paymentId}",
            "/api/documents/{id}/settle",
            "/api/documents/{id}/credit-note",
            "/api/catalog/items",
            "/api/catalog/items/{id}",
            "/api/catalog/groups",
            "/api/catalog/groups/{id}",
            "/api/exchange-rates/{currency}",
            "/api/documents/{id}/pdf",
            "/api/pdf/preview",
            "/api/pdf/design",
            "/api/pdf/design/file",
            "/api/pdf/design/draft/file",
            "/api/pdf/design/draft",
            "/api/pdf/design/draft/preview",
            "/api/pdf/design/activate",
            "/api/pdf/design/versions",
            "/api/pdf/design/versions/{id}/restore",
            "/api/pdf/design/reset",
            "/api/import/isdoc/preview",
            "/api/import/isdoc/confirm",
            "/api/import/csv/preview",
            "/api/import/csv/confirm",
            "/api/import/csv/sample",
            "/api/export/csv",
            "/api/export/accountant",
            "/api/documents/{id}/isdoc",
            "/api/documents/isdoc",
            "/api/settings/email",
            "/api/settings/email/test",
            "/api/settings/email/templates",
            "/api/settings/email/templates/{locale}",
            "/api/settings/email/templates/{locale}/preview",
            "/api/documents/{id}/email",
            "/api/documents/{id}/emails",
            "/api/mcp",
        ] {
            assert!(doc.paths.paths.contains_key(path), "{path}");
        }
        let components = doc.components.expect("components");
        assert!(components.security_schemes.contains_key("bearer"));
        assert!(components.security_schemes.contains_key("cookie"));
        for schema in [
            "CompanyInput",
            "DocDefaults",
            "DocDefaultsInput",
            "RoundingMode",
        ] {
            assert!(components.schemas.contains_key(schema), "{schema}");
        }
        // Required-but-`Option` inputs (422 `required` instead of a serde
        // 400) still read as required.
        let schemas = serde_json::to_value(&components.schemas).expect("json");
        for (schema, fields) in [
            ("TemplateInput", &["subject", "body"][..]),
            (
                "ProfileInput",
                &["vatPayer", "defaultDueDays", "defaultLocale"][..],
            ),
        ] {
            let text = schemas[schema].to_string();
            for f in fields {
                assert!(
                    schemas[schema]["required"]
                        .as_array()
                        .is_some_and(|r| r.iter().any(|v| v == f)),
                    "{schema}.{f}: {text}"
                );
            }
        }
    }
}
