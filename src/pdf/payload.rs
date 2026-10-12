//! The `/data.json` the template reads: every displayed value pre-formatted
//! in the document's locale, every label resolved. Pure.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;

use super::amounts;
use super::format::{self, Locale};
use super::labels::{self, Labels};
use super::spayd::{self, Spayd};
use crate::document::line::{Status, VatMode};
use crate::document::snapshot::{BankSnapshot, PartySnapshot, Totals};
use crate::document::wire::dto::Line;

/// The exchange rate as printed: the rate and, for a ČNB rate, its date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateInfo {
    pub rate: Decimal,
    pub cnb_date: Option<NaiveDate>,
}

/// Everything the payload is built from.
#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub locale: Locale,
    /// Draft → watermark "NÁVRH", cancelled → "STORNO"; only issued gets a QR.
    pub status: Status,
    pub doc_type: &'a str,
    pub vat_mode: VatMode,
    pub number: Option<&'a str>,
    pub issue_date: NaiveDate,
    pub tax_point_date: Option<NaiveDate>,
    pub due_date: NaiveDate,
    pub currency: &'a str,
    pub rate: Option<RateInfo>,
    pub payment_method: &'a str,
    pub variable_symbol: Option<&'a str>,
    pub constant_symbol: Option<&'a str>,
    pub order_ref: Option<&'a str>,
    pub header_note: Option<&'a str>,
    pub footer_note: Option<&'a str>,
    pub correction_reason: Option<&'a str>,
    /// Correction → its original, DDPP → the proforma.
    pub parent_number: Option<&'a str>,
    pub parent_doc_type: Option<&'a str>,
    pub supplier: Option<&'a PartySnapshot>,
    pub customer: Option<&'a PartySnapshot>,
    pub bank: Option<&'a BankSnapshot>,
    pub lines: &'a [Line],
    pub totals: &'a Totals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Row {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Party {
    pub title: String,
    pub name: String,
    pub lines: Vec<String>,
    pub ids: Vec<String>,
    pub contact: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Columns {
    pub description: String,
    pub quantity: String,
    pub unit_price: String,
    pub discount: String,
    pub vat_rate: String,
    pub base: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PdfLine {
    pub kind: &'static str,
    pub description: String,
    pub quantity: Option<String>,
    pub unit_price: Option<String>,
    pub discount: Option<String>,
    pub vat_rate: Option<String>,
    pub base: Option<String>,
    pub strong: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecapRow {
    pub rate: String,
    pub base: String,
    pub vat: String,
    pub total: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recap {
    pub title: String,
    pub rate_note: Option<String>,
    /// Header labels; `rate` is the "Total" label in the `total` row.
    pub columns: RecapRow,
    pub rows: Vec<RecapRow>,
    pub total: RecapRow,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TotalRow {
    pub label: String,
    pub value: String,
    pub strong: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Qr {
    pub image: &'static str,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Assets {
    pub logo: Option<String>,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Footer {
    pub registration: Option<String>,
    pub page_label: String,
    pub issued_by: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    pub locale: &'static str,
    pub draft: bool,
    /// "NÁVRH" / "DRAFT" on a draft, "STORNO" / "CANCELLED" when cancelled.
    pub watermark: Option<String>,
    pub doc_type: String,
    pub vat_mode: &'static str,
    pub title: String,
    pub number: Option<String>,
    pub supplier: Party,
    /// `null` only for a simplified document without a customer.
    pub customer: Option<Party>,
    pub dates: Vec<Row>,
    pub payment: Vec<Row>,
    pub header_note: Option<String>,
    pub footer_note: Option<String>,
    pub reference: Option<String>,
    pub show_vat: bool,
    pub columns: Columns,
    pub has_discount: bool,
    pub lines: Vec<PdfLine>,
    pub vat_recap: Option<Recap>,
    pub vat_recap_czk: Option<Recap>,
    pub totals: Vec<TotalRow>,
    pub legal_note: Option<String>,
    pub paid_note: Option<String>,
    pub qr: Option<Qr>,
    pub assets: Assets,
    pub footer: Footer,
    /// The SPAYD string behind `qr` (rendered into `qr.svg`, not sent).
    #[serde(skip)]
    pub spayd: Option<String>,
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn party(title: &str, p: Option<&PartySnapshot>, l: &Labels, locale: Locale) -> Party {
    let Some(p) = p else {
        return Party {
            title: title.into(),
            name: String::new(),
            lines: vec![],
            ids: vec![],
            contact: vec![],
        };
    };
    let city = format!("{} {}", p.zip.trim(), p.city.trim());
    let lines = [
        non_empty(&p.street),
        non_empty(&city),
        (p.country != "CZ")
            .then(|| labels::country(&p.country, locale))
            .and_then(|c| non_empty(&c)),
    ];
    let ids = [(l.ico, &p.ico), (l.dic, &p.dic)]
        .into_iter()
        .filter_map(|(label, v)| {
            v.as_deref()
                .and_then(non_empty)
                .map(|v| format!("{label}: {v}"))
        });
    let contact = [&p.email, &p.phone, &p.web]
        .into_iter()
        .filter_map(|v| v.as_deref().and_then(non_empty));
    Party {
        title: title.into(),
        name: p.name.clone(),
        lines: lines.into_iter().flatten().collect(),
        ids: ids.collect(),
        contact: contact.collect(),
    }
}

fn row(label: &str, value: String) -> Row {
    Row {
        label: label.into(),
        value,
    }
}

fn dates(i: &Input, l: &Labels) -> Vec<Row> {
    let date = |d| format::date(d, i.locale);
    let mut out = vec![row(l.issue_date, date(i.issue_date))];
    let with_tax_point = i.doc_type != "proforma" && i.vat_mode != VatMode::NonPayer;
    if let Some(d) = i.tax_point_date.filter(|_| with_tax_point) {
        out.push(row(l.tax_point_date, date(d)));
    }
    if i.doc_type == "advance_tax_doc" {
        out.push(row(
            l.payment_date,
            date(i.tax_point_date.unwrap_or(i.issue_date)),
        ));
    } else {
        out.push(row(l.due_date, date(i.due_date)));
    }
    out
}

fn payment(i: &Input, l: &Labels) -> Vec<Row> {
    let mut out = vec![row(
        l.payment_method,
        labels::payment_method(i.payment_method, i.locale).into(),
    )];
    if let Some(b) = i.bank.filter(|_| i.payment_method == "bank_transfer") {
        let fields = [
            (l.account_number, b.account_number.clone()),
            (l.iban, b.iban.as_deref().map(format::iban)),
            (l.bic, b.bic.clone()),
        ];
        for (label, v) in fields {
            if let Some(v) = v.as_deref().and_then(non_empty) {
                out.push(row(label, v));
            }
        }
    }
    for (label, v) in [
        (l.variable_symbol, i.variable_symbol),
        (l.constant_symbol, i.constant_symbol),
        (l.order_ref, i.order_ref),
    ] {
        if let Some(v) = v.and_then(non_empty) {
            out.push(row(label, v));
        }
    }
    out
}

/// No customer block when there is no customer and none can be expected:
/// an issued document without a customer snapshot (a simplified document or
/// one of its corrections), or any draft of / correcting a simplified one.
/// Other drafts keep the empty party.
fn has_customer_block(i: &Input) -> bool {
    i.customer.is_some()
        || (i.status == Status::Draft
            && i.doc_type != "simplified"
            && i.parent_doc_type != Some("simplified"))
}

fn reference(i: &Input) -> Option<String> {
    super::reference::reference(
        i.doc_type,
        i.parent_doc_type,
        i.parent_number?,
        i.correction_reason,
        i.locale,
    )
}

fn qr(i: &Input, title: &str, l: &Labels) -> Option<(Qr, String)> {
    let bank = i.bank?;
    let iban = bank.iban.as_deref();
    if !spayd::applies(
        i.status,
        i.doc_type,
        i.payment_method,
        iban,
        i.totals.payable,
    ) {
        return None;
    }
    let message = match i.number {
        Some(n) => format!("{title} {n}"),
        None => title.to_string(),
    };
    let encoded = Spayd {
        iban: iban?,
        bic: bank.bic.as_deref(),
        amount: i.totals.payable,
        currency: i.currency,
        due_date: i.due_date,
        variable_symbol: i.variable_symbol,
        constant_symbol: i.constant_symbol,
        message: &message,
    }
    .encode();
    Some((
        Qr {
            image: super::design::QR_IMAGE,
            label: l.qr.into(),
        },
        encoded,
    ))
}

/// Build the payload. Fails only on an amount overflow (internal error:
/// stored amounts are far inside the range).
pub fn build(i: &Input) -> anyhow::Result<Payload> {
    let l = labels::labels(i.locale);
    let title = labels::title(i.doc_type, i.vat_mode, i.locale);
    let show_vat = i.vat_mode != VatMode::NonPayer;
    let amounts = amounts::build(i, l, show_vat)?;
    let qr = qr(i, title, l);
    Ok(Payload {
        locale: i.locale.as_str(),
        draft: i.status == Status::Draft,
        watermark: match i.status {
            Status::Draft => Some(l.watermark.to_string()),
            Status::Cancelled => Some(l.cancelled.to_string()),
            Status::Issued => None,
        },
        doc_type: i.doc_type.to_string(),
        vat_mode: i.vat_mode.as_str(),
        title: title.into(),
        number: i.number.map(str::to_string),
        supplier: party(l.supplier, i.supplier, l, i.locale),
        customer: has_customer_block(i).then(|| party(l.customer, i.customer, l, i.locale)),
        dates: dates(i, l),
        payment: payment(i, l),
        header_note: i.header_note.and_then(non_empty),
        footer_note: i.footer_note.and_then(non_empty),
        reference: reference(i),
        show_vat,
        columns: Columns {
            description: l.description.into(),
            quantity: l.quantity.into(),
            unit_price: l.unit_price.into(),
            discount: l.discount.into(),
            vat_rate: l.vat_rate.into(),
            base: if show_vat { l.base } else { l.amount }.into(),
        },
        has_discount: amounts.has_discount,
        lines: amounts.lines,
        vat_recap: amounts.vat_recap,
        vat_recap_czk: amounts.vat_recap_czk,
        totals: amounts.totals,
        legal_note: labels::legal_note(i.vat_mode, i.locale).map(str::to_string),
        paid_note: (i.doc_type == "advance_tax_doc").then(|| l.paid_note.to_string()),
        spayd: qr.as_ref().map(|(_, s)| s.clone()),
        qr: qr.map(|(q, _)| q),
        assets: Assets::default(),
        footer: Footer {
            registration: i
                .supplier
                .and_then(|s| s.registration.as_deref())
                .and_then(non_empty),
            page_label: l.page.into(),
            issued_by: l.issued_by.into(),
        },
    })
}

#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
