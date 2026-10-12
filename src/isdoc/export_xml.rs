//! A stored issued document ([`Source`]) → ISDOC 6.0.2 XML. Pure; element
//! order follows `isdoc-invoice-6.0.2.xsd`.

use base64::Engine as _;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::model::{NS, doc_type_code};
use super::xml::Xml;
use crate::currency::{LOCAL_CURRENCY, is_local};
use crate::document::compute::{RecapRow, round2};
use crate::document::doc_type::DocType;
use crate::document::line::{AdvanceRow, LineData, PaymentMethod, VatMode};
use crate::document::snapshot::{BankSnapshot, PartySnapshot};

/// A deducted advance: a DDPP (`taxed`) or, non-payer form, a proforma.
#[derive(Debug, Clone)]
pub struct Deposit {
    pub number: String,
    pub variable_symbol: String,
    pub taxed: bool,
    pub rows: Vec<AdvanceRow>,
}

/// The PDF beside the ISDOC in an `.isdocx`.
#[derive(Debug, Clone)]
pub struct Supplement {
    pub filename: String,
    pub sha256: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Source {
    pub id: Uuid,
    pub doc_type: DocType,
    pub number: String,
    pub issue_date: NaiveDate,
    pub tax_point_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub currency: String,
    pub rate: Option<Decimal>,
    pub vat_mode: VatMode,
    pub supplier: PartySnapshot,
    pub customer: Option<PartySnapshot>,
    pub bank: Option<BankSnapshot>,
    pub payment_method: PaymentMethod,
    pub variable_symbol: Option<String>,
    pub constant_symbol: Option<String>,
    /// Header note and correction reason.
    pub note: Option<String>,
    pub lines: Vec<LineData>,
    /// The stored recap (net of deducted advances).
    pub recap: Vec<RecapRow>,
    pub rounding: Decimal,
    pub payable: Decimal,
    pub total_czk: Option<Decimal>,
    /// Number and issue date of the corrected document.
    pub original: Option<(String, NaiveDate)>,
    pub deposits: Vec<Deposit>,
    pub supplement: Option<Supplement>,
}

pub(super) fn m(x: Decimal) -> String {
    round2(x).to_string()
}

pub(super) fn n(x: Decimal) -> String {
    x.normalize().to_string()
}

pub(super) struct W<'a> {
    pub(super) x: Xml,
    pub(super) s: &'a Source,
    pub(super) foreign: bool,
    pub(super) fx: Decimal,
}

impl W<'_> {
    pub(super) fn czk(&self, x: Decimal) -> Decimal {
        round2(x * self.fx)
    }

    /// `nameCurr` + `name` (foreign) or `name`; `plain_first` for the
    /// `LegalMonetaryTotal` order.
    pub(super) fn money(&mut self, name: &str, doc: Decimal, czk: Decimal, plain_first: bool) {
        if !self.foreign {
            self.x.leaf(name, m(doc));
            return;
        }
        let curr = format!("{name}Curr");
        if plain_first {
            self.x.leaf(name, m(czk)).leaf(&curr, m(doc));
        } else {
            self.x.leaf(&curr, m(doc)).leaf(name, m(czk));
        }
    }

    fn party(&mut self, wrapper: &'static str, p: &PartySnapshot) {
        let country = if p.country == "CZ" {
            "Česká republika"
        } else {
            p.country.as_str()
        };
        self.x
            .open(wrapper, &[])
            .open("Party", &[])
            .open("PartyIdentification", &[])
            .leaf("ID", p.ico.as_deref().unwrap_or_default())
            .close()
            .open("PartyName", &[])
            .leaf("Name", &p.name)
            .close()
            .open("PostalAddress", &[])
            .leaf("StreetName", &p.street)
            .leaf("BuildingNumber", "")
            .leaf("CityName", &p.city)
            .leaf("PostalZone", &p.zip)
            .open("Country", &[])
            .leaf("IdentificationCode", &p.country)
            .leaf("Name", country)
            .close()
            .close();
        if let Some(dic) = &p.dic {
            self.x
                .open("PartyTaxScheme", &[])
                .leaf("CompanyID", dic)
                .leaf("TaxScheme", "VAT")
                .close();
        }
        if let Some(r) = &p.registration {
            self.x
                .open("RegisterIdentification", &[])
                .leaf("Preformatted", r)
                .close();
        }
        if p.phone.is_some() || p.email.is_some() {
            self.x
                .open("Contact", &[])
                .opt("Telephone", p.phone.as_deref())
                .opt("ElectronicMail", p.email.as_deref())
                .close();
        }
        self.x.close().close();
    }

    pub(super) fn tax_category(&mut self, name: &'static str, rate: Decimal) {
        let mode = self.s.vat_mode;
        let applicable = matches!(mode, VatMode::Standard | VatMode::Exempt);
        self.x.open(name, &[]).leaf("Percent", n(rate));
        if name == "ClassifiedTaxCategory" {
            self.x.leaf("VATCalculationMethod", "0");
        }
        self.x.leaf("VATApplicable", applicable.to_string());
        if name == "TaxCategory" && mode == VatMode::ReverseCharge {
            self.x.leaf("LocalReverseChargeFlag", "true");
        }
        self.x.close();
    }

    fn payment(&mut self) {
        let s = self.s;
        let code = match s.payment_method {
            PaymentMethod::BankTransfer => "42",
            PaymentMethod::Cash => "10",
            PaymentMethod::Card => "48",
            PaymentMethod::Other => "97",
        };
        let bank = s.bank.clone().unwrap_or(BankSnapshot {
            account_number: None,
            iban: None,
            bic: None,
        });
        let account = bank.account_number.unwrap_or_default();
        let (id, bank_code) = account.split_once('/').unwrap_or((account.as_str(), ""));
        self.x
            .open("PaymentMeans", &[])
            .open("Payment", &[])
            .leaf("PaidAmount", m(s.payable));
        self.x
            .leaf("PaymentMeansCode", code)
            .open("Details", &[])
            .leaf(
                "PaymentDueDate",
                s.due_date.unwrap_or(s.issue_date).to_string(),
            )
            .leaf("ID", id)
            .leaf("BankCode", bank_code)
            .leaf("Name", "")
            .leaf("IBAN", bank.iban.unwrap_or_default())
            .leaf("BIC", bank.bic.unwrap_or_default())
            .opt("VariableSymbol", s.variable_symbol.as_deref())
            .opt("ConstantSymbol", s.constant_symbol.as_deref())
            .close()
            .close()
            .close();
    }
}

pub fn render(s: &Source) -> String {
    let foreign = !is_local(&s.currency);
    let fx = if foreign {
        s.rate.unwrap_or(Decimal::ONE)
    } else {
        Decimal::ONE
    };
    let mut w = W {
        x: Xml::new(),
        s,
        foreign,
        fx,
    };
    w.x.open("Invoice", &[("version", "6.0.2"), ("xmlns", NS)])
        .leaf("DocumentType", doc_type_code(s.doc_type).to_string())
        .leaf("ID", &s.number)
        .leaf("UUID", s.id.to_string())
        .leaf("IssuingSystem", "invoice")
        .leaf("IssueDate", s.issue_date.to_string());
    if s.doc_type != DocType::Proforma {
        w.x.opt("TaxPointDate", s.tax_point_date.map(|d| d.to_string()));
    }
    w.x.leaf(
        "VATApplicable",
        s.supplier.vat_payer.unwrap_or(false).to_string(),
    )
    .leaf("ElectronicPossibilityAgreementReference", "")
    .opt("Note", s.note.as_deref())
    .leaf("LocalCurrencyCode", LOCAL_CURRENCY);
    if foreign {
        w.x.leaf("ForeignCurrencyCode", &s.currency);
    }
    w.x.leaf("CurrRate", n(fx)).leaf("RefCurrRate", "1");
    w.party("AccountingSupplierParty", &s.supplier);
    match &s.customer {
        Some(c) => w.party("AccountingCustomerParty", c),
        // A simplified document without a customer (the schema needs one).
        None => {
            w.x.open("AnonymousCustomerParty", &[])
                .leaf("ID", "anonymous")
                .leaf("IDScheme", "")
                .close();
        }
    }
    if let Some((number, date)) = &s.original {
        w.x.open("OriginalDocumentReferences", &[])
            .open("OriginalDocumentReference", &[])
            .leaf("ID", number)
            .leaf("IssueDate", date.to_string())
            .close()
            .close();
    }
    w.lines();
    w.deposits();
    w.totals();
    w.payment();
    if let Some(sup) = &s.supplement {
        let digest = base64::engine::general_purpose::STANDARD.encode(&sup.sha256);
        w.x.open("SupplementsList", &[])
            .open("Supplement", &[("preview", "true")])
            .leaf("Filename", &sup.filename)
            .leaf_attrs(
                "DigestMethod",
                &[("Algorithm", "http://www.w3.org/2001/04/xmlenc#sha256")],
                "",
            )
            .leaf("DigestValue", digest)
            .close()
            .close();
    }
    w.x.finish()
}

/// `manifest.xml` of an `.isdocx`.
pub fn manifest(main: &str) -> String {
    let mut x = Xml::new();
    x.open("manifest", &[("xmlns", super::model::MANIFEST_NS)])
        .leaf_attrs("maindocument", &[("filename", main)], "");
    x.finish()
}

#[cfg(test)]
#[path = "export_xml_tests.rs"]
mod tests;
