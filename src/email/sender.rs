//! Builds the MIME message and sends it over SMTP (`lettre`, tokio + rustls).

use std::time::Duration;

use anyhow::Context as _;
use bytes::Bytes;
use lettre::message::header::{ContentTransferEncoding, ContentType};
use lettre::message::{Attachment, Body, Mailbox, Message, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use uuid::Uuid;

use super::config::{SmtpConfig, TlsMode};
use crate::error::AppError;

/// Total budget of one SMTP exchange (connect + send).
pub const SMTP_TIMEOUT: Duration = Duration::from_secs(30);

/// One file attached to a message.
#[derive(Debug, Clone)]
pub struct File {
    pub filename: String,
    pub content_type: &'static str,
    pub bytes: Bytes,
}

/// A message ready to be built: addresses already parsed.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub reply_to: Option<Mailbox>,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub bcc: Vec<Mailbox>,
    pub subject: String,
    pub body: String,
    pub files: Vec<File>,
}

#[derive(Clone)]
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl Mailer {
    /// Builds the transport (no network).
    pub fn new(cfg: &SmtpConfig) -> anyhow::Result<Self> {
        let tls = match cfg.tls {
            TlsMode::None => Tls::None,
            TlsMode::StartTls => Tls::Required(tls_params(&cfg.host)?),
            TlsMode::Tls => Tls::Wrapper(tls_params(&cfg.host)?),
        };
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
            .port(cfg.port)
            .tls(tls)
            .timeout(Some(SMTP_TIMEOUT));
        if let Some((user, password)) = &cfg.credentials {
            builder =
                builder.credentials(Credentials::new(user.clone(), password.expose().clone()));
        }
        Ok(Self {
            transport: builder.build(),
            from: cfg.from.clone(),
        })
    }

    pub fn from(&self) -> &Mailbox {
        &self.from
    }

    /// `<{uuid}@{domain of FROM}>`.
    pub fn message_id(&self) -> String {
        format!("<{}@{}>", Uuid::new_v4(), self.from.email.domain())
    }

    /// Send `msg` with `message_id`; any SMTP failure or the timeout →
    /// `smtp_failed` with the server's / transport's message.
    pub async fn send(&self, msg: Outgoing, message_id: &str) -> Result<(), AppError> {
        let message = build(&self.from, msg, message_id)?;
        match tokio::time::timeout(SMTP_TIMEOUT, self.transport.send(message)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(AppError::SmtpFailed(e.to_string())),
            Err(_) => Err(AppError::SmtpFailed(format!(
                "timed out after {} s",
                SMTP_TIMEOUT.as_secs()
            ))),
        }
    }
}

fn tls_params(host: &str) -> anyhow::Result<TlsParameters> {
    TlsParameters::new(host.to_string()).context("configure SMTP TLS (INVOICEAPP__SMTP__HOST)")
}

/// The message as sent: `Bcc` goes to the envelope only (lettre drops the
/// header after deriving the envelope).
pub fn build(from: &Mailbox, msg: Outgoing, message_id: &str) -> anyhow::Result<Message> {
    let mut b = Message::builder()
        .from(from.clone())
        .subject(msg.subject)
        .message_id(Some(message_id.to_string()));
    if let Some(r) = msg.reply_to {
        b = b.reply_to(r);
    }
    for m in msg.to {
        b = b.to(m);
    }
    for m in msg.cc {
        b = b.cc(m);
    }
    for m in msg.bcc {
        b = b.bcc(m);
    }
    let text = SinglePart::plain(msg.body);
    let message = if msg.files.is_empty() {
        b.singlepart(text)
    } else {
        let mut parts = MultiPart::mixed().singlepart(text);
        for f in msg.files {
            let content_type = ContentType::parse(f.content_type)
                .with_context(|| format!("content type {}", f.content_type))?;
            // Bytes → Vec reuses the buffer when it is uniquely owned (our
            // freshly built attachments), so the file is not copied here.
            let body = Body::new_with_encoding(Vec::from(f.bytes), ContentTransferEncoding::Base64)
                .map_err(|_| anyhow::anyhow!("base64 attachment encoding refused"))?;
            parts = parts.singlepart(Attachment::new(f.filename).body(body, content_type));
        }
        b.multipart(parts)
    };
    message.context("build e-mail message")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mbox(s: &str) -> Mailbox {
        s.parse().expect("mailbox")
    }

    #[test]
    fn message_has_the_promised_headers_and_parts() {
        let msg = Outgoing {
            reply_to: Some(mbox("firma@example.com")),
            to: vec![mbox("Odběratel <a@example.com>")],
            cc: vec![mbox("c@example.com")],
            bcc: vec![mbox("hidden@example.com")],
            subject: "Faktura 20260001".into(),
            body: "Dobrý den,\nv příloze.".into(),
            files: vec![File {
                filename: "20260001.pdf".into(),
                content_type: "application/pdf",
                bytes: Bytes::from_static(b"%PDF-1.7 test"),
            }],
        };
        let m =
            build(&mbox("Firma <faktury@example.com>"), msg, "<x@example.com>").expect("message");
        let raw = String::from_utf8(m.formatted()).expect("utf-8");
        let head = raw.split("\r\n\r\n").next().expect("headers");
        assert!(head.contains("From: Firma <faktury@example.com>"), "{head}");
        assert!(head.contains("Reply-To: firma@example.com"), "{head}");
        assert!(head.contains("Cc: c@example.com"), "{head}");
        assert!(head.contains("Message-ID: <x@example.com>"), "{head}");
        assert!(head.contains("MIME-Version: 1.0"), "{head}");
        assert!(head.contains("Date: "), "{head}");
        assert!(!raw.contains("hidden@example.com"), "{raw}");
        assert!(raw.contains("Content-Type: application/pdf"), "{raw}");
        assert!(raw.contains("Content-Transfer-Encoding: base64"), "{raw}");
        assert!(raw.contains("charset=utf-8"), "{raw}");
        let rcpt: Vec<String> = m.envelope().to().iter().map(|a| a.to_string()).collect();
        assert_eq!(
            rcpt,
            ["a@example.com", "c@example.com", "hidden@example.com"]
        );
    }

    #[test]
    fn message_without_files_is_single_part() {
        let msg = Outgoing {
            reply_to: None,
            to: vec![mbox("a@example.com")],
            cc: vec![],
            bcc: vec![],
            subject: "Test".into(),
            body: "Hello".into(),
            files: vec![],
        };
        let raw = String::from_utf8(
            build(&mbox("f@example.com"), msg, "<y@example.com>")
                .expect("message")
                .formatted(),
        )
        .expect("utf-8");
        assert!(!raw.contains("multipart"), "{raw}");
        assert!(!raw.contains("Reply-To"), "{raw}");
        assert!(raw.contains("text/plain; charset=utf-8"), "{raw}");
    }

    #[test]
    fn message_id_uses_the_sender_domain() {
        let cfg = SmtpConfig {
            host: "127.0.0.1".into(),
            port: 25,
            tls: TlsMode::None,
            credentials: None,
            from: mbox("Firma <faktury@firma.cz>"),
        };
        let id = Mailer::new(&cfg).expect("mailer").message_id();
        assert!(id.starts_with('<') && id.ends_with("@firma.cz>"), "{id}");
    }
}
