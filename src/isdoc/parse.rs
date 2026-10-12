//! ISDOC XML → [`Parsed`]. Faithful reading only; mapping decisions
//! (direction, sign, VAT mode) live in `plan.rs`.
//!
//! `roxmltree` rejects DTDs by default, so no entity expansion can blow up.

use chrono::NaiveDate;
use roxmltree::{Document, Node};
use rust_decimal::Decimal;

use super::model::{Line, Money, NS, Parsed, Party, Payment, TaxRow, Totals, doc_type_from_code};
use crate::currency::is_local;
use crate::document::line::MAX_INPUT;

/// Entry error code (see `docs/api/isdoc.md`).
pub use crate::import::model::Code;

pub const INVALID_XML: Code = "invalid_xml";
pub const UNSUPPORTED_VERSION: Code = "unsupported_version";
pub const UNSUPPORTED_TYPE: Code = "unsupported_type";
pub const MISSING_FIELD: Code = "missing_field";
pub const INVALID_AMOUNT: Code = "invalid_amount";
pub const UNSUPPORTED_CURRENCY: Code = "unsupported_currency";

/// Longest accepted document number (the `number` / `supplierNumber` limit).
const MAX_NUMBER: usize = 40;

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| {
        c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(NS)
    })
}

fn children<'a, 'i: 'a>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| {
        c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(NS)
    })
}

fn path<'a, 'i>(n: Node<'a, 'i>, names: &[&str]) -> Option<Node<'a, 'i>> {
    names.iter().try_fold(n, |n, name| child(n, name))
}

/// Trimmed text of the element at `names`; empty → `None`.
fn text(n: Node, names: &[&str]) -> Option<String> {
    path(n, names)
        .and_then(|e| e.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn required(n: Node, names: &[&str]) -> Result<String, Code> {
    text(n, names).ok_or(MISSING_FIELD)
}

/// `xs:date`, an optional time-zone suffix ignored.
fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}

/// A decimal with `|x| < 10^12`.
pub fn amount(s: &str) -> Result<Decimal, Code> {
    let d: Decimal = s.trim().parse().map_err(|_| INVALID_AMOUNT)?;
    if d.abs() >= MAX_INPUT {
        return Err(INVALID_AMOUNT);
    }
    Ok(d)
}

fn opt_amount(n: Node, names: &[&str]) -> Result<Option<Decimal>, Code> {
    text(n, names).map(|s| amount(&s)).transpose()
}

/// Currency context: which elements carry the document amounts.
#[derive(Clone, Copy)]
struct Cur {
    foreign: bool,
}

impl Cur {
    /// `name` (local) and `nameCurr` (foreign) of `n`.
    fn money(self, n: Node, name: &str) -> Result<Money, Code> {
        let plain = opt_amount(n, &[name])?.ok_or(MISSING_FIELD)?;
        if !self.foreign {
            return Ok(Money {
                doc: plain,
                czk: None,
            });
        }
        let curr = opt_amount(n, &[&format!("{name}Curr")])?.ok_or(MISSING_FIELD)?;
        Ok(Money {
            doc: curr,
            czk: Some(plain),
        })
    }
}

fn party(n: Node) -> Result<Party, Code> {
    let p = child(n, "Party").ok_or(MISSING_FIELD)?;
    let street = text(p, &["PostalAddress", "StreetName"]).unwrap_or_default();
    let building = text(p, &["PostalAddress", "BuildingNumber"]);
    let registration = text(p, &["RegisterIdentification", "Preformatted"]).or_else(|| {
        let parts: Vec<String> = ["RegisterKeptAt", "RegisterFileRef"]
            .iter()
            .filter_map(|e| text(p, &["RegisterIdentification", e]))
            .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    });
    let country = text(p, &["PostalAddress", "Country", "IdentificationCode"])
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase()))
        .unwrap_or_else(|| "CZ".into());
    Ok(Party {
        name: required(p, &["PartyName", "Name"])?,
        ico: text(p, &["PartyIdentification", "ID"])
            .map(|s| s.chars().filter(|c| !c.is_whitespace()).collect()),
        dic: children(p, "PartyTaxScheme").find_map(|t| text(t, &["CompanyID"])),
        street: match building {
            Some(b) if !street.is_empty() => format!("{street} {b}"),
            Some(b) => b,
            None => street,
        },
        city: text(p, &["PostalAddress", "CityName"]).unwrap_or_default(),
        zip: text(p, &["PostalAddress", "PostalZone"]).unwrap_or_default(),
        country,
        registration,
        email: text(p, &["Contact", "ElectronicMail"]),
        phone: text(p, &["Contact", "Telephone"]),
        web: None,
    })
}

fn line(n: Node, cur: Cur) -> Result<Line, Code> {
    let quantity = opt_amount(n, &["InvoicedQuantity"])?;
    let unit = path(n, &["InvoicedQuantity"])
        .and_then(|q| q.attribute("unitCode"))
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(|u| u.chars().take(20).collect());
    Ok(Line {
        description: text(n, &["Item", "Description"]).unwrap_or_default(),
        quantity,
        unit,
        unit_price: opt_amount(n, &["UnitPrice"])?.unwrap_or_default(),
        base: cur.money(n, "LineExtensionAmount")?,
        before_discount: opt_amount(n, &["LineExtensionAmountBeforeDiscount"])?,
        rate: rate(opt_amount(n, &["ClassifiedTaxCategory", "Percent"])?)?,
    })
}

/// A VAT rate: 0..=100, stored with 2 dp.
fn rate(r: Option<Decimal>) -> Result<Decimal, Code> {
    let r = r.unwrap_or_default();
    if r < Decimal::ZERO || r > Decimal::ONE_HUNDRED || r.round_dp(2) != r {
        return Err(INVALID_AMOUNT);
    }
    Ok(r.normalize())
}

fn bool_text(n: Node, names: &[&str]) -> Option<bool> {
    text(n, names).map(|s| s == "true" || s == "1")
}

fn tax_row(n: Node, cur: Cur) -> Result<TaxRow, Code> {
    Ok(TaxRow {
        rate: rate(opt_amount(n, &["TaxCategory", "Percent"])?)?,
        base: cur.money(n, "DifferenceTaxableAmount")?,
        vat: cur.money(n, "DifferenceTaxAmount")?,
        vat_applicable: bool_text(n, &["TaxCategory", "VATApplicable"]),
        reverse_charge: bool_text(n, &["TaxCategory", "LocalReverseChargeFlag"]) == Some(true),
    })
}

fn totals(n: Node, cur: Cur) -> Result<Totals, Code> {
    let gross = cur.money(n, "TaxInclusiveAmount")?.doc;
    let rounding = if cur.foreign {
        opt_amount(n, &["PayableRoundingAmountCurr"])?
    } else {
        opt_amount(n, &["PayableRoundingAmount"])?
    };
    // Required by the schema; tolerated when missing.
    let paid_deposits = match child(n, "PaidDepositsAmount") {
        Some(_) => cur.money(n, "PaidDepositsAmount")?,
        None => Money::default(),
    };
    Ok(Totals {
        paid_deposits,
        base: cur.money(n, "DifferenceTaxExclusiveAmount")?,
        total: cur.money(n, "DifferenceTaxInclusiveAmount")?,
        gross,
        rounding: rounding.unwrap_or_default(),
        payable: cur.money(n, "PayableAmount")?,
    })
}

fn payment(n: Node) -> Option<Payment> {
    let p = child(n, "Payment")?;
    let d = |name: &str| text(p, &["Details", name]);
    Some(Payment {
        code: text(p, &["PaymentMeansCode"]),
        due_date: d("PaymentDueDate").and_then(|s| date(&s)),
        account: d("ID"),
        bank_code: d("BankCode"),
        iban: d("IBAN"),
        bic: d("BIC"),
        variable_symbol: d("VariableSymbol"),
        constant_symbol: d("ConstantSymbol"),
    })
}

/// `CurrRate / RefCurrRate` (≤ 6 dp) when it is a usable CZK rate.
fn rate_of(root: Node) -> Result<Option<Decimal>, Code> {
    let curr = opt_amount(root, &["CurrRate"])?;
    let reference = opt_amount(root, &["RefCurrRate"])?.unwrap_or(Decimal::ONE);
    Ok(curr
        .filter(|_| reference > Decimal::ZERO)
        .and_then(|c| c.checked_div(reference))
        .map(|r| r.round_dp(6).normalize())
        .filter(|r| *r > Decimal::ZERO && *r != Decimal::ONE))
}

/// The `Supplement` marked as the visual form of the document.
fn preview_file(root: Node) -> Option<String> {
    children(child(root, "SupplementsList")?, "Supplement")
        .find(|s| s.attribute("preview") == Some("true"))
        .and_then(|s| text(s, &["Filename"]))
}

/// [`preview_file`] of raw ISDOC XML (picks the PDF of an `.isdocx` before
/// anything else is decompressed); `None` when it does not parse.
pub fn preview_file_of(xml: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(xml).ok()?;
    let doc = Document::parse(s.trim_start_matches('\u{feff}')).ok()?;
    preview_file(doc.root_element())
}

pub fn parse(xml: &[u8]) -> Result<Parsed, Code> {
    let s = std::str::from_utf8(xml).map_err(|_| INVALID_XML)?;
    let doc = Document::parse(s.trim_start_matches('\u{feff}')).map_err(|_| INVALID_XML)?;
    let root = doc.root_element();
    if root.tag_name().name() != "Invoice" {
        return Err(INVALID_XML);
    }
    // ISDOC 5.x and older live in other namespaces.
    if root.tag_name().namespace() != Some(NS)
        || !root
            .attribute("version")
            .is_some_and(|v| v.starts_with("6."))
    {
        return Err(UNSUPPORTED_VERSION);
    }
    let doc_type =
        doc_type_from_code(&required(root, &["DocumentType"])?).ok_or(UNSUPPORTED_TYPE)?;
    let number = required(root, &["ID"])?;
    if number.chars().count() > MAX_NUMBER {
        return Err(MISSING_FIELD);
    }
    let issue_date = date(&required(root, &["IssueDate"])?).ok_or(MISSING_FIELD)?;
    let local = required(root, &["LocalCurrencyCode"])?.to_ascii_uppercase();
    // Plain amounts are in the local currency; ours is always CZK.
    if !is_local(&local) {
        return Err(UNSUPPORTED_CURRENCY);
    }
    let foreign = text(root, &["ForeignCurrencyCode"])
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| *c != local);
    let currency = foreign.clone().unwrap_or_else(|| local.clone());
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(MISSING_FIELD);
    }
    let cur = Cur {
        foreign: foreign.is_some(),
    };
    let rate = if cur.foreign { rate_of(root)? } else { None };
    let lines = match child(root, "InvoiceLines") {
        Some(l) => children(l, "InvoiceLine")
            .map(|n| line(n, cur))
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let tax = child(root, "TaxTotal").ok_or(MISSING_FIELD)?;
    let recap: Vec<TaxRow> = children(tax, "TaxSubTotal")
        .map(|n| tax_row(n, cur))
        .collect::<Result<_, _>>()?;
    if recap.is_empty() {
        return Err(MISSING_FIELD);
    }
    let totals = totals(child(root, "LegalMonetaryTotal").ok_or(MISSING_FIELD)?, cur)?;
    Ok(Parsed {
        doc_type,
        number,
        issue_date,
        tax_point_date: text(root, &["TaxPointDate"]).and_then(|s| date(&s)),
        vat_applicable: bool_text(root, &["VATApplicable"]).unwrap_or(false),
        note: text(root, &["Note"]),
        currency,
        rate,
        supplier: party(child(root, "AccountingSupplierParty").ok_or(MISSING_FIELD)?)?,
        customer: child(root, "AccountingCustomerParty")
            .map(party)
            .transpose()?,
        original_ref: child(root, "OriginalDocumentReferences")
            .and_then(|r| text(r, &["OriginalDocumentReference", "ID"])),
        lines,
        recap,
        totals,
        payment: child(root, "PaymentMeans").and_then(payment),
    })
}

#[cfg(test)]
#[path = "parse_tests.rs"]
pub(crate) mod tests;
