//! Public registration (base host, `INVOICE__REGISTRATION`), e-mail
//! verification and its resend. Answers never reveal whether an e-mail has
//! an account.

use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

use super::ClientIp;
use crate::app::AppState;
use crate::auth::crypto;
use crate::auth::mail::{self, Mail};
use crate::auth::ratelimit::{MAIL_EMAIL, MAIL_IP};
use crate::auth::users::{self, TokenKind};
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::extract::ApiJson;
use crate::validation as v;

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct RegisterInput {
    pub email: String,
    pub password: String,
    pub display_name: String,
    /// `cs` (default) | `en`: the language of the e-mail.
    pub locale: Option<String>,
}

#[derive(Debug)]
struct Registration {
    email: String,
    password: String,
    display_name: String,
}

impl RegisterInput {
    fn validate(self) -> Result<Registration, AppError> {
        let mut e = FieldErrors::new();
        let email = e.check("email", super::email(&self.email));
        e.check("password", super::password(&self.password));
        let display_name = e.check("displayName", v::required_text(&self.display_name, 100));
        e.into_result()?;
        Ok(Registration {
            email: email.unwrap_or_default(),
            password: self.password,
            display_name: display_name.unwrap_or_default(),
        })
    }
}

/// Both mail limits for one request (per IP and per e-mail).
pub(super) fn mail_limits(state: &AppState, ip: &str, email: &str) -> Result<(), AppError> {
    state.limiter.reserve(&[(MAIL_IP, ip), (MAIL_EMAIL, email)])
}

pub(super) fn link(state: &AppState, page: &str, token: &str) -> String {
    format!("{}/{page}?token={token}", state.public.base_url())
}

#[utoipa::path(
    post,
    path = "/api/auth/register",
    tag = "auth",
    request_body = RegisterInput,
    responses(
        (status = 202, description = "Accepted (whether or not the e-mail is new)"),
        (status = 404, description = "Registration disabled", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody),
        (status = 429, description = "`rate_limited`", body = ErrorBody),
    )
)]
pub async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(input): ApiJson<RegisterInput>,
) -> Result<StatusCode, AppError> {
    if !state.registration {
        return Err(AppError::NotFound);
    }
    let locale = mail::locale(input.locale.as_deref());
    let reg = input.validate()?;
    mail_limits(&state, &ip, &reg.email)?;
    // Hashed for a known e-mail too, so both answers take the same time.
    let hash = crypto::hash_password(reg.password).await?;
    let existing = users::find_by_email(&state.db, &reg.email).await?;
    let created = match existing {
        Some(_) => None,
        None => users::create(&state.db, &reg.email, &reg.display_name, Some(hash)).await?,
    };
    let mail = match created {
        Some(user) => Mail::Verify(link(
            &state,
            "verify",
            &users::issue_token(&state.db, user.id, TokenKind::Verify).await?,
        )),
        None => {
            // A disabled account gets nothing (same as a password-reset request).
            let Some(user) = users::find_by_email(&state.db, &reg.email)
                .await?
                .filter(|u| !u.disabled)
            else {
                return Ok(StatusCode::ACCEPTED);
            };
            Mail::AlreadyRegistered(link(
                &state,
                "reset",
                &users::issue_token(&state.db, user.id, TokenKind::Reset).await?,
            ))
        }
    };
    mail::send(&state, &reg.email, mail, locale);
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct TokenBody {
    pub token: String,
}

#[utoipa::path(
    post,
    path = "/api/auth/verify",
    tag = "auth",
    request_body = TokenBody,
    responses(
        (status = 204, description = "E-mail verified"),
        (status = 422, description = "`token: invalid` (unknown, expired or used)", body = ErrorBody),
    )
)]
pub async fn verify(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<TokenBody>,
) -> Result<StatusCode, AppError> {
    users::consume_token(
        &state.db,
        body.token.trim(),
        TokenKind::Verify,
        |txn, user_id| async move {
            users::mark_verified(&txn, user_id).await?;
            Ok(txn)
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct EmailBody {
    pub email: String,
    /// `cs` (default) | `en`.
    pub locale: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/auth/verify/resend",
    tag = "auth",
    request_body = EmailBody,
    responses(
        (status = 202, description = "Accepted (always)"),
        (status = 429, description = "`rate_limited`", body = ErrorBody),
    )
)]
pub async fn resend(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<EmailBody>,
) -> Result<StatusCode, AppError> {
    let email = users::normalize_email(&body.email);
    mail_limits(&state, &ip, &email)?;
    let Some(user) = users::find_by_email(&state.db, &email)
        .await?
        .filter(|u| u.email_verified_at.is_none() && !u.disabled)
    else {
        return Ok(StatusCode::ACCEPTED);
    };
    let token = users::issue_token(&state.db, user.id, TokenKind::Verify).await?;
    let locale = mail::locale(body.locale.as_deref());
    mail::send(
        &state,
        &user.email,
        Mail::Verify(link(&state, "verify", &token)),
        locale,
    );
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_registration() {
        let err = RegisterInput::default().validate().expect_err("empty");
        let r = format!("{err:?}");
        assert!(r.contains("\"email\": \"required\""), "{r}");
        assert!(r.contains("\"password\": \"too_short\""), "{r}");
        assert!(r.contains("\"displayName\": \"required\""), "{r}");
        let ok = RegisterInput {
            email: " Jana@Example.com".into(),
            password: "correct horse battery".into(),
            display_name: " Jana ".into(),
            locale: None,
        }
        .validate()
        .expect("valid");
        assert_eq!(ok.email, "jana@example.com");
        assert_eq!(ok.display_name, "Jana");
    }
}
