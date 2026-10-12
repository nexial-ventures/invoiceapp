//! What the Pohoda and Money S3 writers read the same way from a stored
//! document: its label and description, numbers that must fit, the tax and
//! accounting dates, the CZK recap in the standard rate classes. Pure.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::document::doc_type::DocType;
use crate::document::entity::document;
use crate::document::line::Direction;
use crate::export::source::Source;

/// The standard Czech rates (základní, snížená).
pub const HIGH: Decimal = Decimal::from_parts(21, 0, 0, false, 0);
pub const LOW: Decimal = Decimal::from_parts(12, 0, 0, false, 0);

/// The label of a document type (Czech, as the UI names the types).
pub fn label(doc_type: DocType, issued: bool) -> &'static str {
    match (doc_type, issued) {
        (DocType::Invoice, true) => "Faktura",
        (DocType::Invoice, false) => "Přijatá faktura",
        (DocType::CreditNote, true) => "Dobropis",
        (DocType::CreditNote, false) => "Přijatý dobropis",
        (DocType::DebitNote, true) => "Vrubopis",
        (DocType::DebitNote, false) => "Přijatý vrubopis",
        (DocType::AdvanceTaxDoc, true) => "Daňový doklad k platbě",
        (DocType::AdvanceTaxDoc, false) => "Přijatý daňový doklad k platbě",
        (DocType::AdvanceCreditNote, true) => "Opravný daňový doklad k platbě",
        (DocType::AdvanceCreditNote, false) => "Přijatý opravný doklad k platbě",
        (DocType::Simplified, true) => "Zjednodušený daňový doklad",
        (DocType::Simplified, false) => "Přijatý zjednodušený daňový doklad",
        _ => "Doklad",
    }
}

/// `s` when it fits `max` characters (an XSD `maxLength`), else an error:
/// a number is never cut.
pub fn fits<'a>(what: &str, s: &'a str, max: usize) -> anyhow::Result<&'a str> {
    anyhow::ensure!(s.chars().count() <= max, "{what} longer than {max}: {s}");
    Ok(s)
}

/// The first `max` characters (descriptions and address lines the
/// programs keep shorter).
pub fn cut(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

/// The number the file names a document by (issued: ours, received: the
/// supplier's, else our internal one), for messages and descriptions.
pub fn shown_number(doc: &document::Model) -> Option<&str> {
    match doc.direction {
        Direction::Issued => doc.number.as_deref(),
        _ => doc.supplier_number.as_deref().or(doc.number.as_deref()),
    }
}

/// Label + shown number (+ ` k {original}` for a linked correction or
/// DDPP), uncut: `Dobropis DB-1 k FA-1`.
pub fn text(s: &Source, doc_type: DocType, issued: bool) -> String {
    let original = s.parent.and_then(|p| match issued {
        true => p.number.as_deref(),
        false => p.supplier_number.as_deref().or(p.number.as_deref()),
    });
    let mut text = label(doc_type, issued).to_string();
    for (sep, n) in [(" ", shown_number(s.doc)), (" k ", original)] {
        if let Some(n) = n {
            text.push_str(sep);
            text.push_str(n);
        }
    }
    text
}

/// `(tax date, accounting date)`: DUZP (received: else the received date);
/// issued account on the tax date, received on the received date (else
/// the tax date).
pub fn dates(doc: &document::Model, issued: bool) -> (Option<NaiveDate>, Option<NaiveDate>) {
    let tax = doc.tax_point_date.or(doc.received_date.filter(|_| !issued));
    let accounting = match issued {
        true => tax,
        false => doc.received_date.or(tax),
    };
    (tax, accounting)
}

/// A recap in the standard rate classes, amounts positive as stored.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Rates {
    /// 0 % (and every base of a VAT-free document): base + VAT.
    pub none: Option<Decimal>,
    /// 12 %: `(base, VAT)`.
    pub low: Option<(Decimal, Decimal)>,
    /// 21 %: `(base, VAT)`.
    pub high: Option<(Decimal, Decimal)>,
    /// Any other rate: `(rate, base, VAT)`, one entry per rate, recap order.
    pub other: Vec<(Decimal, Decimal, Decimal)>,
}

fn add(slot: &mut Option<(Decimal, Decimal)>, base: Decimal, vat: Decimal) {
    let (b, v) = slot.get_or_insert((Decimal::ZERO, Decimal::ZERO));
    *b += base;
    *v += vat;
}

/// The recap `(rate, base, VAT)` by class: 21 % high, 12 % low, 0 % none,
/// the rest per rate. `vat_free` (exempt / non-payer): every base goes to
/// none — there is no VAT, and a rate class would make the program tax it.
pub fn rates(recap: &[(Decimal, Decimal, Decimal)], vat_free: bool) -> Rates {
    let mut s = Rates::default();
    for &(rate, base, vat) in recap {
        let rate = rate.normalize();
        if vat_free || rate.is_zero() {
            *s.none.get_or_insert(Decimal::ZERO) += base + vat;
        } else if rate == HIGH {
            add(&mut s.high, base, vat);
        } else if rate == LOW {
            add(&mut s.low, base, vat);
        } else if let Some(o) = s.other.iter_mut().find(|o| o.0 == rate) {
            o.1 += base;
            o.2 += vat;
        } else {
            s.other.push((rate, base, vat));
        }
    }
    s
}

/// An amount with two decimals and a `.` decimal point (both XSDs' decimal
/// types); never `-0.00`.
pub fn amount(x: Decimal) -> String {
    let x = x.round_dp(2);
    if x.is_zero() {
        "0.00".into()
    } else {
        format!("{x:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::test_doc::{d, date, doc};

    #[test]
    fn classes() {
        let recap = [
            (d("21.00"), d("1000"), d("210")),
            (d("12"), d("100"), d("12")),
            (d("0"), d("50"), d("0")),
            (d("10"), d("10"), d("1")),
            (d("10.00"), d("20"), d("2")),
            (d("15"), d("100"), d("15")),
        ];
        let r = rates(&recap, false);
        assert_eq!(r.high, Some((d("1000"), d("210"))));
        assert_eq!(r.low, Some((d("100"), d("12"))));
        assert_eq!(r.none, Some(d("50")));
        assert_eq!(
            r.other,
            [(d("10"), d("30"), d("3")), (d("15"), d("100"), d("15"))]
        );
        let free = rates(&recap[..2], true);
        assert_eq!(
            free,
            Rates {
                none: Some(d("1322")),
                ..Rates::default()
            }
        );
    }

    #[test]
    fn amounts() {
        assert_eq!(amount(d("1210")), "1210.00");
        assert_eq!(amount(d("-121.5")), "-121.50");
        assert_eq!(amount(d("0") * d("-1")), "0.00");
    }

    #[test]
    fn numbers_and_dates() {
        assert!(fits("number", "FA-1", 4).is_ok());
        let e = fits("number", "FA-10", 4).expect_err("long");
        assert_eq!(e.to_string(), "number longer than 4: FA-10");
        assert_eq!(cut("  Žluťoučký kůň ", 5), "Žluťo");

        let mut doc = doc();
        doc.tax_point_date = None;
        assert_eq!(dates(&doc, true), (None, None));
        assert_eq!(dates(&doc, false), (Some(date(3, 2)), Some(date(3, 2))));
        doc.tax_point_date = Some(date(3, 1));
        doc.received_date = None;
        assert_eq!(dates(&doc, false), (Some(date(3, 1)), Some(date(3, 1))));
        doc.direction = Direction::Received;
        doc.supplier_number = None;
        assert_eq!(shown_number(&doc), Some("2026000001"), "internal number");
    }
}
