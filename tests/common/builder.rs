//! [`TestApp::builder`]: the one way to assemble a test router. Every
//! `router*` / `PdfEnv::router*` / `EmailEnv::router*` / `google::app*`
//! helper is a thin wrapper over it.

use std::time::Duration;

use axum::Router;
use invoice::app::AppState;
use invoice::ares::DEFAULT_ARES_URL;
use invoice::auth::google::{Google, GoogleConfig};
use invoice::cnb::DEFAULT_CNB_URL;
use invoice::email::{Mailer, SmtpConfig};
use invoice::pdf::PdfRoot;
use invoice::secret::Secret;
use invoice::storage::Storage;
use sea_orm::DatabaseConnection;

use super::mdcast::PdfEnv;
use super::oidc::{CLIENT_ID, CLIENT_SECRET};

pub struct TestApp;

impl TestApp {
    /// Defaults: ARES / ČNB at the real URLs (never called), a fresh mock
    /// mdcast, the binary's shared fs storage, no SMTP, no Google,
    /// registration off, `X-Forwarded-For` not trusted.
    pub fn builder(db: &DatabaseConnection) -> TestAppBuilder {
        TestAppBuilder {
            db: db.clone(),
            ares: DEFAULT_ARES_URL.into(),
            cnb: DEFAULT_CNB_URL.into(),
            mdcast: None,
            storage: None,
            smtp: None,
            google: None,
            jwks_refresh: invoice::auth::google::provider::JWKS_REFRESH,
            registration: false,
            trust_forwarded: false,
            edits: Vec::new(),
        }
    }
}

type Edit = Box<dyn FnOnce(&mut AppState)>;

#[must_use]
pub struct TestAppBuilder {
    db: DatabaseConnection,
    ares: String,
    cnb: String,
    mdcast: Option<String>,
    storage: Option<Storage>,
    smtp: Option<SmtpConfig>,
    google: Option<String>,
    jwks_refresh: Duration,
    registration: bool,
    trust_forwarded: bool,
    edits: Vec<Edit>,
}

impl TestAppBuilder {
    /// ARES at `url` (a local mock server).
    pub fn ares(mut self, url: &str) -> Self {
        self.ares = url.into();
        self
    }

    /// ČNB at `url` (a local mock server).
    pub fn cnb(mut self, url: &str) -> Self {
        self.cnb = url.into();
        self
    }

    /// Render against `env`'s mock mdcast into its private storage.
    pub fn mdcast(self, env: &PdfEnv) -> Self {
        self.mdcast_url(&env.url)
            .storage(env.storage.storage.clone())
    }

    /// mdcast at `url` (a mock, or a dead port).
    pub fn mdcast_url(mut self, url: &str) -> Self {
        self.mdcast = Some(url.into());
        self
    }

    pub fn storage(mut self, storage: Storage) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Send e-mail through the mock SMTP server of `config`.
    pub fn smtp(mut self, config: &SmtpConfig) -> Self {
        self.smtp = Some(config.clone());
        self
    }

    /// Sign in with Google against the (mock) OIDC provider at `issuer`.
    pub fn google(mut self, issuer: &str) -> Self {
        self.google = Some(issuer.into());
        self
    }

    /// Minimum interval between JWKS refetches (with [`Self::google`]).
    pub fn jwks_refresh(mut self, d: Duration) -> Self {
        self.jwks_refresh = d;
        self
    }

    pub fn registration(mut self, on: bool) -> Self {
        self.registration = on;
        self
    }

    pub fn trust_forwarded(mut self, on: bool) -> Self {
        self.trust_forwarded = on;
        self
    }

    /// Escape hatch: change anything else of the state, after the options
    /// above are applied.
    pub fn state(mut self, edit: impl FnOnce(&mut AppState) + 'static) -> Self {
        self.edits.push(Box::new(edit));
        self
    }

    pub fn build(self) -> Router {
        self.build_with_pdf().0
    }

    /// [`Self::build`] plus the app's PDF service (its shared design cache
    /// is inspectable).
    pub fn build_with_pdf(self) -> (Router, PdfRoot) {
        let mdcast = self.mdcast.unwrap_or_else(|| super::mdcast::spawn().0);
        let storage = self.storage.unwrap_or_else(super::shared_storage);
        let pdf = super::pdf_service(&mdcast, storage);
        let mut state = super::state(self.db, &self.ares, &self.cnb, pdf.clone());
        state.email = self.smtp.map(|c| Mailer::new(&c).expect("mailer"));
        state.google = self.google.map(|issuer| {
            let cfg = GoogleConfig {
                client_id: CLIENT_ID.into(),
                client_secret: Secret::new(CLIENT_SECRET.into()),
                issuer,
            };
            Google::with_jwks_refresh(cfg, self.jwks_refresh).expect("google client")
        });
        state.registration = self.registration;
        state.trust_forwarded = self.trust_forwarded;
        for edit in self.edits {
            edit(&mut state);
        }
        (super::app(state), pdf)
    }
}
