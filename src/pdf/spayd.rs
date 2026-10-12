//! SPAYD ("QR platba") payment string and its QR code as SVG.

use anyhow::Context as _;
use chrono::NaiveDate;
use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};
use rust_decimal::Decimal;

use crate::document::compute::round2;
use crate::document::doc_type::DocType;
use crate::document::line::PaymentMethod;
use crate::document::line::Status;

/// Maximum length of the `MSG` field.
const MSG_MAX: usize = 60;

/// Whether the document gets a QR code: an issued (not draft, not
/// cancelled) invoice, proforma, debit note or simplified document paid by
/// bank transfer to an IBAN with something left to pay.
pub fn applies(
    status: Status,
    doc_type: &str,
    payment_method: &str,
    iban: Option<&str>,
    payable: Decimal,
) -> bool {
    status == Status::Issued
        && matches!(
            DocType::parse(doc_type),
            Some(DocType::Invoice | DocType::Proforma | DocType::DebitNote | DocType::Simplified)
        )
        && payment_method == PaymentMethod::BankTransfer.as_str()
        && iban.is_some_and(|i| !i.trim().is_empty())
        && payable > Decimal::ZERO
}

pub struct Spayd<'a> {
    pub iban: &'a str,
    pub bic: Option<&'a str>,
    pub amount: Decimal,
    pub currency: &'a str,
    pub due_date: NaiveDate,
    pub variable_symbol: Option<&'a str>,
    pub constant_symbol: Option<&'a str>,
    /// `"{title} {number}"`, folded to ASCII and cut to 60 characters.
    pub message: &'a str,
}

/// `*` separates SPAYD fields, so it may not appear inside a value.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '*' && !c.is_whitespace())
        .collect()
}

impl Spayd<'_> {
    pub fn encode(&self) -> String {
        let mut acc = clean(self.iban);
        if let Some(bic) = self.bic.map(clean).filter(|b| !b.is_empty()) {
            acc = format!("{acc}+{bic}");
        }
        let mut out = format!(
            "SPD*1.0*ACC:{acc}*AM:{}*CC:{}*DT:{}",
            round2(self.amount),
            clean(self.currency),
            self.due_date.format("%Y%m%d")
        );
        for (key, value) in [
            ("X-VS", self.variable_symbol),
            ("X-KS", self.constant_symbol),
        ] {
            if let Some(v) = value.map(clean).filter(|v| !v.is_empty()) {
                out.push_str(&format!("*{key}:{v}"));
            }
        }
        let msg: String = ascii_fold(self.message)
            .replace('*', "")
            .chars()
            .take(MSG_MAX)
            .collect();
        out.push_str(&format!("*MSG:{}", msg.trim_end()));
        out
    }
}

/// Czech (and common Central European) diacritics → ASCII; typographic
/// dashes → `-`; anything else outside ASCII is dropped.
pub fn ascii_fold(s: &str) -> String {
    const FROM: &str = "áäčďéěëíĺľňóôöŕřšťúůüýžÁÄČĎÉĚËÍĹĽŇÓÔÖŔŘŠŤÚŮÜÝŽ";
    const TO: &str = "aacdeeeillnooorrstuuuyzAACDEEEILLNOOORRSTUUUYZ";
    s.chars()
        .filter_map(|c| match c {
            _ if c.is_ascii() => Some(c),
            '–' | '—' => Some('-'),
            '\u{a0}' => Some(' '),
            _ => FROM
                .chars()
                .position(|f| f == c)
                .and_then(|i| TO.chars().nth(i)),
        })
        .collect()
}

/// The QR code as a standalone SVG document.
pub fn qr_svg(payload: &str) -> anyhow::Result<String> {
    let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M)
        .context("encode SPAYD as a QR code")?;
    Ok(code
        .render::<svg::Color<'_>>()
        .min_dimensions(256, 256)
        .quiet_zone(false)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 15).expect("date")
    }

    #[test]
    fn full_string() {
        let s = Spayd {
            iban: "CZ65 0800 0000 1920 0014 5399",
            bic: Some("GIBACZPX"),
            amount: d("1210"),
            currency: "CZK",
            due_date: day(),
            variable_symbol: Some("20260001"),
            constant_symbol: Some("0308"),
            message: "Faktura – daňový doklad 20260001",
        };
        assert_eq!(
            s.encode(),
            "SPD*1.0*ACC:CZ6508000000192000145399+GIBACZPX*AM:1210.00*CC:CZK*DT:20261015\
             *X-VS:20260001*X-KS:0308*MSG:Faktura - danovy doklad 20260001"
        );
    }

    #[test]
    fn optional_fields_and_stars_are_dropped() {
        let s = Spayd {
            iban: "CZ65*0800",
            bic: None,
            amount: d("99.5"),
            currency: "EUR",
            due_date: day(),
            variable_symbol: None,
            constant_symbol: Some(""),
            message: "Zálohová faktura Z*1",
        };
        assert_eq!(
            s.encode(),
            "SPD*1.0*ACC:CZ650800*AM:99.50*CC:EUR*DT:20261015*MSG:Zalohova faktura Z1"
        );
    }

    #[test]
    fn message_is_cut_to_sixty_ascii_chars() {
        let long = "Žluťoučký kůň úpěl ďábelské ódy ".repeat(4);
        let s = Spayd {
            iban: "CZ65",
            bic: None,
            amount: d("1"),
            currency: "CZK",
            due_date: day(),
            variable_symbol: None,
            constant_symbol: None,
            message: &long,
        };
        let encoded = s.encode();
        let msg = encoded.split("*MSG:").nth(1).expect("MSG");
        assert!(msg.len() <= 60, "{msg}");
        assert!(msg.is_ascii());
        assert!(msg.starts_with("Zlutoucky kun upel dabelske ody"));
    }

    #[test]
    fn fold() {
        assert_eq!(ascii_fold("Příliš žluťoučký kůň"), "Prilis zlutoucky kun");
        assert_eq!(ascii_fold("ÚŮ – ß€"), "UU - ");
    }

    #[test]
    fn qr_condition() {
        let ok = |draft: bool, t, m, iban, p: &str| {
            let status = if draft { Status::Draft } else { Status::Issued };
            applies(status, t, m, iban, d(p))
        };
        assert!(ok(false, "invoice", "bank_transfer", Some("CZ65"), "1"));
        assert!(ok(false, "proforma", "bank_transfer", Some("CZ65"), "1"));
        assert!(ok(false, "debit_note", "bank_transfer", Some("CZ65"), "1"));
        assert!(ok(false, "simplified", "bank_transfer", Some("CZ65"), "1"));
        assert!(!ok(
            false,
            "advance_credit_note",
            "bank_transfer",
            Some("CZ65"),
            "1"
        ));
        assert!(!ok(true, "invoice", "bank_transfer", Some("CZ65"), "1"));
        assert!(!applies(
            Status::Cancelled,
            "invoice",
            "bank_transfer",
            Some("CZ65"),
            d("1")
        ));
        assert!(!ok(
            false,
            "credit_note",
            "bank_transfer",
            Some("CZ65"),
            "1"
        ));
        assert!(!ok(
            false,
            "advance_tax_doc",
            "bank_transfer",
            Some("CZ65"),
            "1"
        ));
        assert!(!ok(false, "invoice", "cash", Some("CZ65"), "1"));
        assert!(!ok(false, "invoice", "bank_transfer", None, "1"));
        assert!(!ok(false, "invoice", "bank_transfer", Some(" "), "1"));
        assert!(!ok(false, "invoice", "bank_transfer", Some("CZ65"), "0"));
    }

    #[test]
    fn svg_is_rendered() {
        let svg = qr_svg("SPD*1.0*ACC:CZ65*AM:1.00*CC:CZK").expect("qr");
        assert!(svg.contains("<svg"), "{svg}");
    }
}
