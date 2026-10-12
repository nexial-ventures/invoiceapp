//! Unit tests of `error.rs`.

use super::*;
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;

async fn body(err: AppError) -> (StatusCode, serde_json::Value) {
    let resp = err.into_response();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4096)
        .await
        .expect("read body");
    (status, serde_json::from_slice(&bytes).expect("json"))
}

#[test]
fn maps_variants_to_status_and_code() {
    let cases = [
        (
            AppError::Unauthorized,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        (AppError::NotFound, StatusCode::NOT_FOUND, "not_found"),
        (
            AppError::BadRequest("x".into()),
            StatusCode::BAD_REQUEST,
            "bad_request",
        ),
        (
            AppError::field("name", "required"),
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation",
        ),
        (
            AppError::Conflict("x".into()),
            StatusCode::CONFLICT,
            "conflict",
        ),
        (AppError::MfaDisabled, StatusCode::CONFLICT, "mfa_disabled"),
        (AppError::EmailTaken, StatusCode::CONFLICT, "email_taken"),
        (AppError::Duplicate, StatusCode::CONFLICT, "duplicate"),
        (
            AppError::AresNotFound,
            StatusCode::NOT_FOUND,
            "ares_not_found",
        ),
        (
            AppError::AresUnavailable(anyhow::anyhow!("timeout")),
            StatusCode::BAD_GATEWAY,
            "ares_unavailable",
        ),
        (
            AppError::DocumentLocked,
            StatusCode::CONFLICT,
            "document_locked",
        ),
        (
            AppError::InvalidState,
            StatusCode::CONFLICT,
            "invalid_state",
        ),
        (
            AppError::CnbUnavailable(anyhow::anyhow!("timeout")),
            StatusCode::BAD_GATEWAY,
            "cnb_unavailable",
        ),
        (
            AppError::AdvanceSettled,
            StatusCode::CONFLICT,
            "advance_settled",
        ),
        (
            AppError::AdvanceInUse,
            StatusCode::CONFLICT,
            "advance_in_use",
        ),
        (
            AppError::CatalogItemInUse,
            StatusCode::CONFLICT,
            "catalog_item_in_use",
        ),
        (
            AppError::CategoryInUse,
            StatusCode::CONFLICT,
            "category_in_use",
        ),
        (AppError::NumberTaken, StatusCode::CONFLICT, "number_taken"),
        (AppError::LastOwner, StatusCode::CONFLICT, "last_owner"),
        (
            AppError::ExceedsOriginal,
            StatusCode::CONFLICT,
            "exceeds_original",
        ),
        (AppError::PdfMissing, StatusCode::NOT_FOUND, "pdf_missing"),
        (
            AppError::TooLarge,
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
        ),
        (
            AppError::PdfUnavailable("down".into()),
            StatusCode::SERVICE_UNAVAILABLE,
            "pdf_unavailable",
        ),
        (
            AppError::StorageUnavailable("down".into()),
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
        ),
        (
            AppError::PdfRenderFailed("typst".into()),
            StatusCode::BAD_GATEWAY,
            "pdf_render_failed",
        ),
        (
            AppError::from(anyhow::anyhow!("boom")),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
        ),
        (
            AppError::from(DbErr::Custom("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
        ),
    ];
    for (err, status, code) in cases {
        assert_eq!(err.status_and_code(), (status, code), "{err}");
    }
}

#[test]
fn unauthorized_sets_www_authenticate() {
    let resp = AppError::Unauthorized.into_response();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers().get(header::WWW_AUTHENTICATE),
        Some(&HeaderValue::from_static("Bearer"))
    );
}

#[tokio::test]
async fn auth_errors() {
    for (err, status, code) in [
        (
            AppError::InvalidCredentials,
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
        ),
        (AppError::Forbidden, StatusCode::FORBIDDEN, "forbidden"),
        (
            AppError::EmailUnverified,
            StatusCode::FORBIDDEN,
            "email_unverified",
        ),
        (
            AppError::SpaceCreationDisabled,
            StatusCode::FORBIDDEN,
            "space_creation_disabled",
        ),
        (AppError::SpaceLimit, StatusCode::FORBIDDEN, "space_limit"),
        (
            AppError::EmailDisabled,
            StatusCode::SERVICE_UNAVAILABLE,
            "email_disabled",
        ),
        (AppError::Csrf, StatusCode::FORBIDDEN, "csrf"),
        (AppError::Busy, StatusCode::SERVICE_UNAVAILABLE, "busy"),
        (
            AppError::RateLimited(42),
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
        ),
    ] {
        let (s, json) = body(err).await;
        assert_eq!(s, status);
        assert_eq!(json, serde_json::json!({ "code": code }));
    }
    let resp = AppError::RateLimited(42).into_response();
    assert_eq!(
        resp.headers().get(header::RETRY_AFTER),
        Some(&HeaderValue::from_static("42"))
    );
    let resp = AppError::InvalidCredentials.into_response();
    assert!(resp.headers().get(header::WWW_AUTHENTICATE).is_none());
    let resp = AppError::PdfBusy.into_response();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        resp.headers().get(header::RETRY_AFTER),
        Some(&HeaderValue::from_static("10"))
    );
    let (_, json) = body(AppError::PdfBusy).await;
    assert_eq!(
        json,
        serde_json::json!({ "code": "pdf_unavailable", "detail": "busy" })
    );
}

#[tokio::test]
async fn internal_error_does_not_leak_detail() {
    let (_, json) = body(AppError::from(anyhow::anyhow!("password=hunter2"))).await;
    assert_eq!(json, serde_json::json!({ "code": "internal" }));
    let (_, json) = body(AppError::AresUnavailable(anyhow::anyhow!("dns fail"))).await;
    assert_eq!(json, serde_json::json!({ "code": "ares_unavailable" }));
    let (_, json) = body(AppError::Conflict("contacts_pkey".into())).await;
    assert_eq!(json, serde_json::json!({ "code": "conflict" }));
}

#[tokio::test]
async fn validation_lists_fields() {
    let mut errors = FieldErrors::new();
    errors.add("name", "required");
    errors.add("name", "too_long");
    errors.add("ico", "invalid_ico");
    let (status, json) = body(AppError::Validation(errors)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json,
        serde_json::json!({
            "code": "validation",
            "fields": { "ico": "invalid_ico", "name": "required" }
        })
    );
}

#[test]
fn field_errors_check_and_result() {
    let mut errors = FieldErrors::new();
    assert_eq!(errors.check("a", Ok::<_, &'static str>(1)), Some(1));
    assert!(errors.clone().into_result().is_ok());
    assert_eq!(errors.check::<()>("b", Err("invalid")), None);
    assert!(matches!(errors.into_result(), Err(AppError::Validation(_))));
}

#[tokio::test]
async fn pdf_errors_carry_detail_only_for_render_failures() {
    let (status, json) = body(AppError::PdfRenderFailed("unknown variable: foo".into())).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        json,
        serde_json::json!({ "code": "pdf_render_failed", "detail": "unknown variable: foo" })
    );
    let (status, json) = body(AppError::PdfUpstream("<html>stack trace</html>".into())).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(json, serde_json::json!({ "code": "pdf_render_failed" }));
    let (status, json) = body(AppError::PdfUnavailable("connect refused 10.0.0.1".into())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json, serde_json::json!({ "code": "pdf_unavailable" }));
    let (status, json) = body(AppError::StorageUnavailable("bucket s3://x denied".into())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json, serde_json::json!({ "code": "storage_unavailable" }));
}

#[tokio::test]
async fn email_errors() {
    assert_eq!(
        body(AppError::SmtpNotConfigured).await,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({ "code": "smtp_not_configured" })
        )
    );
    assert_eq!(
        body(AppError::SmtpFailed("permanent error (550): no".into())).await,
        (
            StatusCode::BAD_GATEWAY,
            serde_json::json!({ "code": "smtp_failed", "detail": "permanent error (550): no" })
        )
    );
    assert_eq!(
        body(AppError::TemplateInvalid {
            field: "body",
            detail: "line 3: undefined value".into()
        })
        .await,
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            serde_json::json!({
                "code": "template_invalid",
                "fields": { "body": "template_invalid" },
                "detail": "line 3: undefined value"
            })
        )
    );
}

#[tokio::test]
async fn validation_detail_names_the_part() {
    let (status, json) = body(AppError::field_detail(
        "file",
        "missing_column",
        "supplier_number",
    ))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json,
        serde_json::json!({
            "code": "validation",
            "fields": { "file": "missing_column" },
            "detail": "supplier_number"
        })
    );
    let (_, plain) = body(AppError::field("file", "empty")).await;
    assert!(plain.get("detail").is_none(), "{plain}");
}

#[tokio::test]
async fn mfa_errors() {
    assert_eq!(
        body(AppError::MfaRequired {
            account_created: false
        })
        .await,
        (
            StatusCode::FORBIDDEN,
            serde_json::json!({ "code": "mfa_required" })
        )
    );
    assert_eq!(
        body(AppError::MfaRequired {
            account_created: true
        })
        .await,
        (
            StatusCode::FORBIDDEN,
            serde_json::json!({ "code": "mfa_required", "detail": "account_created" })
        )
    );
    assert_eq!(
        body(AppError::MfaInvalid).await,
        (
            StatusCode::UNAUTHORIZED,
            serde_json::json!({ "code": "mfa_invalid" })
        )
    );
    assert_eq!(
        body(AppError::MfaEnabled).await,
        (
            StatusCode::CONFLICT,
            serde_json::json!({ "code": "mfa_enabled" })
        )
    );
}

#[tokio::test]
async fn design_errors() {
    let unprocessable = StatusCode::UNPROCESSABLE_ENTITY;
    assert_eq!(
        body(AppError::DesignInvalid("en: unknown variable".into())).await,
        (
            unprocessable,
            serde_json::json!({ "code": "design_invalid", "detail": "en: unknown variable" })
        )
    );
    assert_eq!(
        body(AppError::DraftEmpty).await,
        (unprocessable, serde_json::json!({ "code": "draft_empty" }))
    );
    for (err, code) in [
        (AppError::DesignBusy, "design_busy"),
        (AppError::DraftChanged, "draft_changed"),
        (AppError::VersionIncomplete, "version_incomplete"),
    ] {
        assert_eq!(
            body(err).await,
            (StatusCode::CONFLICT, serde_json::json!({ "code": code }))
        );
    }
    assert_eq!(
        body(AppError::QuotaExceeded("49.5 / 50 MB".into())).await,
        (
            unprocessable,
            serde_json::json!({ "code": "quota_exceeded", "detail": "49.5 / 50 MB" })
        )
    );
}
