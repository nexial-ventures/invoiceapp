//! Stored documents → Money S3 native XML (`MoneyData`, UTF-8, no
//! namespace). Pure: the export pipeline ([`super::export`]) loads the
//! [`Source`]s and assembles what this writes.

use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};

use super::doc::{cut, dates, fits, rates, shown_number, text};
use super::export::Ctx;
use super::money_summary as summary;
use super::settings::CodeRow;
use crate::currency::is_local;
use crate::document::doc_type::DocType;
use crate::document::line::{PaymentMethod, VatMode};
use crate::export::source::{Kind, Source, czk_recap, snapshot};
use crate::isdoc::xml::Xml;

/// Longest `Doklad`; a longer number is left out and Money numbers the
/// document from the series (`Rada`).
pub const MAX_DOKLAD: usize = 10;

const LISTS: usize = 4;

/// The `MoneyData` list a document goes to, in XSD order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Received,
    Issued,
    ReceivedDpp,
    IssuedDpp,
}

impl List {
    const ALL: [List; LISTS] = [
        List::Received,
        List::Issued,
        List::ReceivedDpp,
        List::IssuedDpp,
    ];

    fn of(doc_type: DocType, issued: bool) -> anyhow::Result<Self> {
        Ok(match (doc_type, issued) {
            (DocType::AdvanceTaxDoc | DocType::AdvanceCreditNote, true) => List::IssuedDpp,
            (DocType::AdvanceTaxDoc | DocType::AdvanceCreditNote, false) => List::ReceivedDpp,
            (
                DocType::Invoice | DocType::Simplified | DocType::CreditNote | DocType::DebitNote,
                true,
            ) => List::Issued,
            (
                DocType::Invoice | DocType::Simplified | DocType::CreditNote | DocType::DebitNote,
                false,
            ) => List::Received,
            (other, _) => anyhow::bail!("{} is not exported to Money S3", other.as_str()),
        })
    }

    /// `(list element, item element)`.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            List::Received => ("SeznamFaktPrij", "FaktPrij"),
            List::Issued => ("SeznamFaktVyd", "FaktVyd"),
            List::ReceivedDpp => ("SeznamFaktPrij_DPP", "FaktPrij_DPP"),
            List::IssuedDpp => ("SeznamFaktVyd_DPP", "FaktVyd_DPP"),
        }
    }

    fn ddpp(self) -> bool {
        matches!(self, List::ReceivedDpp | List::IssuedDpp)
    }
}

/// The XML declaration and the open `MoneyData`.
pub fn head(ctx: &Ctx, today: NaiveDate) -> String {
    let description = format!("Export z Invoice {}–{}", ctx.from, ctx.to);
    let today = today.to_string();
    let mut attrs = Vec::new();
    if let Some(ico) = &ctx.ico {
        attrs.push(("ICAgendy", ico.as_str()));
    }
    attrs.extend([("description", description.as_str()), ("ExpDate", &today)]);
    let mut x = Xml::fragment();
    x.open("MoneyData", &attrs);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}\n",
        x.unclosed()
    )
}

/// The items of a file, collected per [`List`] (`MoneyData` holds each
/// list once).
#[derive(Debug, Default)]
pub struct Lists([String; LISTS]);

impl Lists {
    pub fn push(&mut self, list: List, item: &str) {
        self.0[list as usize].push_str(item);
    }

    /// The whole file: [`head`], every non-empty list and the end.
    pub fn finish(self, ctx: &Ctx, today: NaiveDate) -> String {
        let mut out = head(ctx, today);
        for list in List::ALL {
            let items = &self.0[list as usize];
            if !items.is_empty() {
                let (name, _) = list.names();
                for part in ["<", name, ">\n", items, "</", name, ">\n"] {
                    out.push_str(part);
                }
            }
        }
        out.push_str("</MoneyData>\n");
        out
    }
}

/// `DodOdb` from the counterparty snapshot, cut to the XSD lengths; no
/// snapshot (a contactless simplified document) → nothing.
fn partner(x: &mut Xml, stored: &Option<serde_json::Value>) -> anyhow::Result<()> {
    let Some(p) = snapshot(stored)? else {
        return Ok(());
    };
    let some = |v: String| Some(v).filter(|v| !v.is_empty());
    let street = some(cut(&p.street, 50));
    let city = some(cut(&p.city, 40));
    let zip = some(cut(&p.zip, 10));
    // `KodStatu` is the ISO code (exactly 2); `Stat` would be the name.
    let country = Some(p.country.trim()).filter(|c| c.chars().count() == 2);
    x.open("DodOdb", &[])
        .opt("ObchNazev", some(p.name.trim().to_string()));
    if street.is_some() || city.is_some() || zip.is_some() || country.is_some() {
        x.open("ObchAdresa", &[])
            .opt("Ulice", street)
            .opt("Misto", city)
            .opt("PSC", zip)
            .opt("KodStatu", country)
            .close();
    }
    x.opt("ICO", p.ico.as_deref().map(|v| cut(v, 10)).and_then(some))
        .opt("DIC", p.dic.as_deref().map(|v| cut(v, 20)).and_then(some))
        .opt("PlatceDPH", p.vat_payer.map(|b| b.to_string()))
        .close();
    Ok(())
}

/// `Uhrada` (free text ≤ 20) of a stored payment method; `other` → none.
fn payment(method: PaymentMethod) -> Option<&'static str> {
    match method {
        PaymentMethod::BankTransfer => Some("převodem"),
        PaymentMethod::Cash => Some("hotově"),
        PaymentMethod::Card => Some("kartou"),
        PaymentMethod::Other => None,
    }
}

/// `Kurs` (`castkaType`: 4 decimals at most): the stored rate, else the
/// rate the recap implies (CZK total / currency total), so Money never
/// picks its own; `None` only when neither exists (a zero total).
fn kurs(stored: Option<Decimal>, czk: Decimal, foreign: Decimal) -> Option<Decimal> {
    stored
        .or_else(|| (!foreign.is_zero()).then(|| czk / foreign))
        .map(|r| {
            r.round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero)
                .normalize()
        })
}

fn sum(recap: &[(Decimal, Decimal, Decimal)]) -> Decimal {
    recap.iter().map(|(_, base, vat)| base + vat).sum()
}

/// One document in its list's item element (`fakturaType`, elements in
/// XSD order, empty values left out). An error makes the document
/// unexportable (a number too long for Money, too many rates, an
/// undecodable snapshot…).
pub fn item(s: &Source, ctx: &Ctx) -> anyhow::Result<(List, String)> {
    let doc = s.doc;
    let Kind {
        issued,
        doc_type,
        vat_mode,
        ..
    } = Kind::of(doc);
    let list = List::of(doc_type, issued)?;
    let vat_free = matches!(vat_mode, VatMode::Exempt | VatMode::NonPayer);
    let home_recap = czk_recap(doc, s.recap)?;
    let home = rates(&home_recap, vat_free);
    let codes: Option<&CodeRow> = ctx.settings.money.row(doc.direction, doc_type);
    let code = |f: fn(&CodeRow) -> &Option<String>| codes.and_then(|c| f(c).as_ref());
    // A received document without the deduction never takes the
    // deductible code; without its own code KodDPH is left out.
    let classification = match !issued && !doc.vat_deductible {
        true => code(|c| &c.classification_vat_non_deductible),
        false => code(|c| &c.classification_vat),
    };
    let (tax_date, accounting_date) = dates(doc, issued);
    let date = |d: Option<NaiveDate>| d.map(|d| d.to_string());
    let sign = Decimal::from(doc_type.sign());

    let mut x = Xml::fragment();
    x.open(list.names().1, &[]).opt(
        "Doklad",
        doc.number
            .as_deref()
            .filter(|n| n.chars().count() <= MAX_DOKLAD),
    );
    if issued && let Some(n) = &doc.number {
        x.leaf("EvCisDokl", fits("number", n, 50)?);
    }
    x.opt("Rada", code(|c| &c.number_series))
        .leaf("Popis", cut(&text(s, doc_type, issued), 50))
        .leaf("Vystaveno", doc.issue_date.to_string())
        .opt("DatUcPr", date(accounting_date))
        .opt("PlnenoDPH", date(tax_date))
        .opt("Splatno", date(doc.due_date))
        .opt("Doruceno", date(doc.received_date.filter(|_| !issued)))
        .opt("KodDPH", classification);
    if issued && doc_type == DocType::Simplified {
        x.leaf("ZjednD", "true");
    }
    if let Some(vs) = &doc.variable_symbol {
        x.leaf("VarSymbol", fits("variable symbol", vs, 20)?);
    }
    if !issued && let Some(n) = shown_number(doc) {
        x.leaf("PrijatDokl", fits("supplier number", n, 50)?);
    }
    x.leaf("Druh", if list.ddpp() { "D" } else { "N" });
    if doc_type.sign() < 0 {
        x.leaf("Dobropis", "true");
    }
    x.opt("Uhrada", payment(doc.payment_method))
        .opt("PredKontac", code(|c| &c.accounting))
        .leaf("SazbaDPH1", "12")
        .leaf("SazbaDPH2", "21");
    if is_local(&doc.currency) {
        summary::write(&mut x, &home, doc.rounding, sign)?;
    } else {
        let foreign: Vec<_> = s
            .recap
            .iter()
            .map(|r| (r.vat_rate, r.base, r.vat))
            .collect();
        let kurs = kurs(doc.exchange_rate, sum(&home_recap), sum(&foreign));
        // The rounding is in the currency; the CZK summary carries it at
        // the written rate, so both summaries describe the same payable.
        let home_rounding = kurs.map_or(Decimal::ZERO, |k| {
            (doc.rounding * k).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        });
        summary::write(&mut x, &home, home_rounding, sign)?;
        x.open("Valuty", &[])
            .open("Mena", &[])
            .leaf("Kod", fits("currency", &doc.currency, 4)?)
            .leaf("Mnozstvi", "1")
            .opt("Kurs", kurs.map(|k| k.to_string()))
            .close();
        summary::write(&mut x, &rates(&foreign, vat_free), doc.rounding, sign)?;
        x.close();
    }
    partner(
        &mut x,
        match issued {
            true => &doc.customer_snapshot,
            false => &doc.supplier_snapshot,
        },
    )?;
    let mut out = x.finish();
    out.push('\n');
    Ok((list, out))
}

#[cfg(test)]
#[path = "money_tests.rs"]
mod tests;
