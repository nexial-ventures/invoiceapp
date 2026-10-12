//! Request validation of the send route: addresses, subject, body,
//! attachment flags.

use lettre::message::Mailbox;
use serde::Deserialize;
use utoipa::ToSchema;

use super::templates::SUBJECT_MAX;
use crate::error::FieldErrors;

pub const MAX_RECIPIENTS: usize = 10;
pub const SEND_BODY_MAX: usize = 100_000;

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendInput {
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub attach_pdf: bool,
    #[serde(default)]
    pub attach_isdoc: bool,
}

/// One recipient list, parsed: the trimmed texts (as logged) and mailboxes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recipients {
    pub texts: Vec<String>,
    pub mailboxes: Vec<Mailbox>,
}

/// `addr@x` or `Name <addr@x>`.
pub fn parse_address(s: &str) -> Option<Mailbox> {
    s.trim().parse().ok()
}

fn recipients(errors: &mut FieldErrors, field: &str, list: &[String]) -> Recipients {
    let mut out = Recipients::default();
    for (i, raw) in list.iter().enumerate() {
        match parse_address(raw) {
            Some(m) => {
                out.texts.push(raw.trim().to_string());
                out.mailboxes.push(m);
            }
            None => errors.add(&format!("{field}.{i}"), "invalid"),
        }
    }
    out
}

/// A validated send request.
#[derive(Debug, Clone)]
pub struct Validated {
    pub to: Recipients,
    pub cc: Recipients,
    pub bcc: Recipients,
    pub subject: String,
    pub body: String,
}

/// `pdf_available` = the document has a PDF to attach.
pub fn validate(input: &SendInput, pdf_available: bool) -> Result<Validated, FieldErrors> {
    let mut errors = FieldErrors::new();
    if input.to.is_empty() {
        errors.add("to", "required");
    }
    if input.to.len() + input.cc.len() + input.bcc.len() > MAX_RECIPIENTS {
        errors.add("to", "too_long");
    }
    let to = recipients(&mut errors, "to", &input.to);
    let cc = recipients(&mut errors, "cc", &input.cc);
    let bcc = recipients(&mut errors, "bcc", &input.bcc);
    let subject = input.subject.trim();
    if subject.is_empty() {
        errors.add("subject", "required");
    } else if subject.chars().count() > SUBJECT_MAX {
        errors.add("subject", "too_long");
    } else if subject.contains(['\r', '\n']) {
        errors.add("subject", "invalid");
    }
    if input.body.chars().count() > SEND_BODY_MAX {
        errors.add("body", "too_long");
    }
    if input.attach_pdf && !pdf_available {
        errors.add("attachPdf", "invalid");
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Validated {
        to,
        cc,
        bcc,
        subject: subject.to_string(),
        body: input.body.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(to: &[&str]) -> SendInput {
        SendInput {
            to: to.iter().map(|s| s.to_string()).collect(),
            subject: "Faktura".into(),
            body: "Dobrý den".into(),
            ..Default::default()
        }
    }

    #[test]
    fn addresses() {
        assert!(parse_address("a@example.com").is_some());
        let m = parse_address(" Odběratel a.s. <a@example.com> ").expect("named");
        assert_eq!(m.name.as_deref(), Some("Odběratel a.s."));
        for bad in [
            "",
            "a",
            "a@",
            "@x.cz",
            "a b@x.cz",
            "<a@x.cz",
            "a@x.cz, b@x.cz",
        ] {
            assert!(parse_address(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn valid_request_is_trimmed() {
        let mut i = input(&[" a@example.com "]);
        i.subject = "  Faktura 1 ".into();
        let v = validate(&i, false).expect("valid");
        assert_eq!(v.to.texts, ["a@example.com"]);
        assert_eq!(v.subject, "Faktura 1");
    }

    #[test]
    fn field_errors() {
        let mut i = input(&[]);
        i.cc = vec!["ok@example.com".into(), "bad".into()];
        i.bcc = vec!["also bad".into()];
        i.subject = "a\nb".into();
        i.body = "x".repeat(SEND_BODY_MAX + 1);
        i.attach_pdf = true;
        let e = validate(&i, false).expect_err("invalid");
        assert_eq!(e.get("to"), Some("required"));
        assert_eq!(e.get("cc.1"), Some("invalid"));
        assert_eq!(e.get("cc.0"), None);
        assert_eq!(e.get("bcc.0"), Some("invalid"));
        assert_eq!(e.get("subject"), Some("invalid"));
        assert_eq!(e.get("body"), Some("too_long"));
        assert_eq!(e.get("attachPdf"), Some("invalid"));

        let e = validate(&input(&["nope"]), true).expect_err("invalid");
        assert_eq!(e.get("to.0"), Some("invalid"));

        let mut empty = input(&["a@example.com"]);
        empty.subject = " ".into();
        assert_eq!(
            validate(&empty, true).expect_err("x").get("subject"),
            Some("required")
        );
        empty.subject = "x".repeat(SUBJECT_MAX + 1);
        assert_eq!(
            validate(&empty, true).expect_err("x").get("subject"),
            Some("too_long")
        );

        let many: Vec<String> = (0..=MAX_RECIPIENTS)
            .map(|n| format!("a{n}@example.com"))
            .collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            validate(&input(&refs), true).expect_err("x").get("to"),
            Some("too_long")
        );
    }
}
