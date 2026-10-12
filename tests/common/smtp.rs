//! An in-process mock SMTP server (plain text, no TLS, no AUTH): records
//! every accepted message (envelope + DATA) and can be told to reject.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{Method, StatusCode};
use base64::Engine as _;
use invoice::email::SmtpConfig;
use invoice::email::config::TlsMode;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

pub const FROM: &str = "Dodavatel <faktury@dodavatel.cz>";

/// One accepted message.
#[derive(Debug, Clone)]
pub struct Mail {
    pub mail_from: String,
    pub rcpt: Vec<String>,
    /// The DATA, dot-unstuffed, without the final `.`.
    pub data: String,
}

impl Mail {
    /// The top-level header block.
    pub fn headers(&self) -> &str {
        self.data.split("\r\n\r\n").next().unwrap_or_default()
    }

    /// The value of top-level header `name` (unfolded).
    pub fn header(&self, name: &str) -> Option<String> {
        let prefix = format!("{}:", name.to_ascii_lowercase());
        let mut lines = self.headers().split("\r\n").peekable();
        while let Some(line) = lines.next() {
            if line.to_ascii_lowercase().starts_with(&prefix) {
                let mut value = line[prefix.len()..].trim().to_string();
                while let Some(next) = lines.peek().filter(|l| l.starts_with([' ', '\t'])) {
                    value.push(' ');
                    value.push_str(next.trim());
                    lines.next();
                }
                return Some(value);
            }
        }
        None
    }

    /// The decoded body of the attachment named `filename` and its part headers.
    pub fn attachment(&self, filename: &str) -> Option<(String, Vec<u8>)> {
        let marker = format!("filename=\"{filename}\"");
        let start = self.data.find(&marker)?;
        let part_start = self.data[..start].rfind("\r\n--")?;
        let rest = &self.data[part_start..];
        let (head, body) = rest.split_once("\r\n\r\n")?;
        let end = body.find("\r\n--")?;
        let b64: String = body[..end].split_whitespace().collect();
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        Some((head.to_string(), bytes))
    }

    /// Filenames of every attachment, in order.
    pub fn attachment_names(&self) -> Vec<String> {
        self.data
            .split("filename=\"")
            .skip(1)
            .filter_map(|s| s.split_once('"').map(|(n, _)| n.to_string()))
            .collect()
    }
}

#[derive(Default)]
struct Inner {
    mails: Vec<Mail>,
    /// `Some(reply)` → every `MAIL FROM` is answered with it.
    reject: Option<String>,
}

#[derive(Clone, Default)]
pub struct SmtpMock(Arc<Mutex<Inner>>);

impl SmtpMock {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().expect("mock SMTP state")
    }

    pub fn mails(&self) -> Vec<Mail> {
        self.lock().mails.clone()
    }

    pub fn last(&self) -> Mail {
        self.mails().pop().expect("a message reached the mock")
    }

    /// Answer `MAIL FROM` with `reply` (e.g. `550 5.7.1 rejected`).
    pub fn reject_with(&self, reply: &str) {
        self.lock().reject = Some(reply.to_string());
    }
}

async fn session(stream: tokio::net::TcpStream, mock: SmtpMock) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read);
    write.write_all(b"220 mock ESMTP\r\n").await?;
    let (mut from, mut rcpt) = (String::new(), Vec::new());
    let mut line = String::new();
    loop {
        line.clear();
        if lines.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let cmd = line.trim_end().to_string();
        let upper = cmd.to_ascii_uppercase();
        let reply = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            "250 mock".to_string()
        } else if upper.starts_with("MAIL FROM:") {
            match mock.lock().reject.clone() {
                Some(r) => r,
                None => {
                    from = cmd[10..].trim().to_string();
                    rcpt.clear();
                    "250 OK".into()
                }
            }
        } else if upper.starts_with("RCPT TO:") {
            rcpt.push(cmd[8..].trim().to_string());
            "250 OK".into()
        } else if upper == "DATA" {
            write.write_all(b"354 go ahead\r\n").await?;
            let mut data = String::new();
            loop {
                line.clear();
                if lines.read_line(&mut line).await? == 0 {
                    return Ok(());
                }
                if line == ".\r\n" {
                    break;
                }
                data.push_str(line.strip_prefix('.').unwrap_or(&line));
            }
            mock.lock().mails.push(Mail {
                mail_from: from.clone(),
                rcpt: std::mem::take(&mut rcpt),
                data,
            });
            "250 queued".into()
        } else if upper == "QUIT" {
            write.write_all(b"221 bye\r\n").await?;
            return Ok(());
        } else {
            "250 OK".into()
        };
        write.write_all(format!("{reply}\r\n").as_bytes()).await?;
    }
}

/// Start the mock on a free port; the config points at it (`TLS=none`).
pub async fn spawn() -> (SmtpConfig, SmtpMock) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock SMTP");
    let port = listener.local_addr().expect("mock address").port();
    let mock = SmtpMock::default();
    let m = mock.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(session(stream, m.clone()));
        }
    });
    let cfg = SmtpConfig {
        host: "127.0.0.1".into(),
        port,
        tls: TlsMode::None,
        credentials: None,
        from: FROM.parse().expect("from mailbox"),
    };
    (cfg, mock)
}

/// A mock mdcast + private fs storage + mock SMTP.
pub struct EmailEnv {
    pub pdf: super::mdcast::PdfEnv,
    pub smtp: SmtpMock,
    pub config: SmtpConfig,
}

impl EmailEnv {
    pub async fn new() -> Self {
        let (config, smtp) = spawn().await;
        Self {
            pdf: super::mdcast::PdfEnv::new(),
            smtp,
            config,
        }
    }

    /// The state sending through the mock, for tests that change more of it.
    pub fn state(&self, db: sea_orm::DatabaseConnection) -> invoice::app::AppState {
        let pdf = super::pdf_service(&self.pdf.url, self.pdf.storage.storage.clone());
        let mut state = super::state(
            db,
            invoice::ares::DEFAULT_ARES_URL,
            invoice::cnb::DEFAULT_CNB_URL,
            pdf,
        );
        state.email = Some(invoice::email::Mailer::new(&self.config).expect("mailer"));
        state
    }

    fn router_with(&self, db: sea_orm::DatabaseConnection, configured: bool) -> Router {
        let mut state = self.state(db);
        if !configured {
            state.email = None;
        }
        super::app(state)
    }

    /// The app sending through the mock.
    pub fn router(&self, db: sea_orm::DatabaseConnection) -> Router {
        self.router_with(db, true)
    }

    /// Same storage and mdcast, no SMTP configured.
    pub fn unconfigured(&self, db: sea_orm::DatabaseConnection) -> Router {
        self.router_with(db, false)
    }
}

pub const COMPANY_EMAIL: &str = "firma@dodavatel.cz";
pub const CONTACT_EMAIL: &str = "fakturace@odberatel.cz";

/// A payer company with an e-mail, a contact with an e-mail and a CZK
/// account → an issuable draft body.
pub async fn issuable_with_emails(app: &Router) -> serde_json::Value {
    let (status, body) = super::call(
        app,
        Method::PUT,
        "/api/settings/company",
        Some(json!({
            "name": "Dodavatel s.r.o.", "ico": "44444443", "dic": "CZ44444443", "vatPayer": true,
            "street": "Hlavní 1", "city": "Praha", "zip": "11000", "country": "CZ",
            "email": COMPANY_EMAIL, "defaultDueDays": 14, "defaultLocale": "cs"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let contact = super::documents::create_contact(app, json!({ "email": CONTACT_EMAIL })).await;
    super::documents::create_bank(app, "CZK").await;
    json!({ "contactId": contact, "issueDate": "2026-10-01",
            "lines": [super::documents::item("10", "100", "21")] })
}

/// A valid send body to the contact; `extra` overrides fields.
pub fn send_body(extra: Value) -> Value {
    let mut b = json!({
        "to": [CONTACT_EMAIL], "cc": [], "bcc": [], "subject": "Faktura 20260001",
        "body": "Dobrý den,\nv příloze faktura.", "attachPdf": false, "attachIsdoc": false
    });
    if let (Some(o), Some(e)) = (b.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    b
}

pub async fn post_email(app: &axum::Router, doc: &str, body: Value) -> (StatusCode, Value) {
    super::call(
        app,
        Method::POST,
        &format!("/api/documents/{doc}/email"),
        Some(body),
    )
    .await
}

pub async fn history(app: &axum::Router, doc: &str) -> (StatusCode, Value) {
    super::call(
        app,
        Method::GET,
        &format!("/api/documents/{doc}/emails"),
        None,
    )
    .await
}
