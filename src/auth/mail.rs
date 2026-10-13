//! Account e-mails (verification, "already registered", password reset,
//! space invitation): plain text in cs / en, sent through the instance SMTP
//! in a spawned task. A send failure is logged and never changes the
//! response (no account enumeration; the user can ask again).

use crate::app::AppState;
use crate::email::input::parse_address;
use crate::email::sender::Outgoing;
use crate::pdf::format::Locale;
use crate::space::Role;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mail {
    /// `{baseUrl}/verify?token=…`
    Verify(String),
    /// Registration with a known e-mail: `{baseUrl}/reset?token=…`.
    AlreadyRegistered(String),
    /// `{baseUrl}/reset?token=…`
    Reset(String),
    /// An invitation into a space: `{space url}/invite?token=…`.
    Invite {
        url: String,
        space: String,
        inviter: String,
        role: Role,
    },
}

impl Mail {
    /// For logs (never the link: it carries a token).
    fn kind(&self) -> &'static str {
        match self {
            Self::Verify(_) => "verify",
            Self::AlreadyRegistered(_) => "already_registered",
            Self::Reset(_) => "reset",
            Self::Invite { .. } => "invite",
        }
    }
}

/// `cs` unless the request asked for `en`.
pub fn locale(raw: Option<&str>) -> Locale {
    raw.and_then(|l| Locale::parse(l.trim()))
        .unwrap_or(Locale::Cs)
}

/// (subject, body).
pub fn render(mail: &Mail, locale: Locale) -> (String, String) {
    match (mail, locale) {
        (Mail::Verify(url), Locale::Cs) => (
            "Potvrďte svůj e-mail".into(),
            format!(
                "Dobrý den,\n\npro dokončení registrace v aplikaci InvoiceApp potvrďte svůj e-mail:\n\n{url}\n\n\
                 Odkaz platí 24 hodin. Pokud jste se neregistrovali, tento e-mail ignorujte.\n"
            ),
        ),
        (Mail::Verify(url), Locale::En) => (
            "Confirm your e-mail".into(),
            format!(
                "Hello,\n\nto finish your InvoiceApp registration, confirm your e-mail address:\n\n{url}\n\n\
                 The link is valid for 24 hours. If you did not register, ignore this e-mail.\n"
            ),
        ),
        (Mail::AlreadyRegistered(url), Locale::Cs) => (
            "Účet už existuje".into(),
            format!(
                "Dobrý den,\n\nněkdo se pokusil zaregistrovat s touto adresou, ale účet s ní už máte.\n\
                 Pokud jste zapomněli heslo, nastavte si nové:\n\n{url}\n\n\
                 Odkaz platí 1 hodinu. Pokud jste o nic nežádali, tento e-mail ignorujte.\n"
            ),
        ),
        (Mail::AlreadyRegistered(url), Locale::En) => (
            "You already have an account".into(),
            format!(
                "Hello,\n\nsomeone tried to register with this address, but you already have an account.\n\
                 If you forgot your password, set a new one:\n\n{url}\n\n\
                 The link is valid for 1 hour. If you did not ask for anything, ignore this e-mail.\n"
            ),
        ),
        (Mail::Reset(url), Locale::Cs) => (
            "Obnovení hesla".into(),
            format!(
                "Dobrý den,\n\nnové heslo k aplikaci InvoiceApp si nastavíte zde:\n\n{url}\n\n\
                 Odkaz platí 1 hodinu. Pokud jste o obnovení nežádali, tento e-mail ignorujte.\n"
            ),
        ),
        (Mail::Reset(url), Locale::En) => (
            "Password reset".into(),
            format!(
                "Hello,\n\nset a new InvoiceApp password here:\n\n{url}\n\n\
                 The link is valid for 1 hour. If you did not ask for a reset, ignore this e-mail.\n"
            ),
        ),
        (
            Mail::Invite {
                url,
                space,
                inviter,
                role,
            },
            Locale::Cs,
        ) => (
            format!("{inviter} vás zve do {space}"),
            format!(
                "Dobrý den,\n\n{inviter} vás zve do {space} v aplikaci InvoiceApp s rolí {}.\n\
                 Pozvánku přijmete zde:\n\n{url}\n\n\
                 Odkaz platí 7 dní. Pokud pozvánku nečekáte, tento e-mail ignorujte.\n",
                role_cs(*role)
            ),
        ),
        (
            Mail::Invite {
                url,
                space,
                inviter,
                role,
            },
            Locale::En,
        ) => (
            format!("{inviter} invites you to {space}"),
            format!(
                "Hello,\n\n{inviter} invites you to {space} in InvoiceApp as {}.\n\
                 Accept the invitation here:\n\n{url}\n\n\
                 The link is valid for 7 days. If you did not expect it, ignore this e-mail.\n",
                role.as_str()
            ),
        ),
    }
}

fn role_cs(role: Role) -> &'static str {
    match role {
        Role::Owner => "vlastník",
        Role::Admin => "správce",
        Role::Member => "člen",
        Role::Accountant => "účetní",
    }
}

/// Send `mail` to `to` in the background: the request answers at once in
/// every branch (whether an account exists must not show in the timing);
/// failures (no SMTP, refused) are logged only. `true` = queued.
pub fn send(state: &AppState, to: &str, mail: Mail, locale: Locale) -> bool {
    if state.email_disabled {
        tracing::warn!(
            kind = mail.kind(),
            "account e-mail skipped: INVOICEAPP__EMAIL_DISABLED"
        );
        return false;
    }
    let Some(mailer) = state.email.clone() else {
        tracing::error!("account e-mail not sent: SMTP is not configured");
        return false;
    };
    let Some(to) = parse_address(to) else {
        tracing::warn!("account e-mail not sent: the stored address does not parse");
        return false;
    };
    let (subject, body) = render(&mail, locale);
    let msg = Outgoing {
        reply_to: None,
        to: vec![to],
        cc: vec![],
        bcc: vec![],
        // Names in the subject are user input: no header line breaks.
        subject: subject.replace(char::is_control, " "),
        body,
        files: vec![],
    };
    tokio::spawn(async move {
        if let Err(e) = mailer.send(msg, &mailer.message_id()).await {
            tracing::error!(error = %e, "account e-mail not sent");
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_every_mail_in_both_locales() {
        let url = "https://invoiceapp.cz/verify?token=abc".to_string();
        for mail in [
            Mail::Verify(url.clone()),
            Mail::AlreadyRegistered(url.clone()),
            Mail::Reset(url.clone()),
            Mail::Invite {
                url: url.clone(),
                space: "Acme".into(),
                inviter: "Jana".into(),
                role: Role::Member,
            },
        ] {
            for l in [Locale::Cs, Locale::En] {
                let (subject, body) = render(&mail, l);
                assert!(!subject.is_empty());
                if let Mail::Invite { .. } = mail {
                    assert!(
                        subject.contains("Jana") && subject.contains("Acme"),
                        "{subject}"
                    );
                    assert!(body.contains("Jana") && body.contains("Acme"), "{body}");
                }
                assert!(body.contains(&url), "{body}");
            }
        }
        assert_eq!(locale(Some("en")), Locale::En);
        assert_eq!(locale(Some("de")), Locale::Cs);
        assert_eq!(locale(None), Locale::Cs);
    }
}
