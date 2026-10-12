//! An ISDOC document as read from the XML, before any mapping decision.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::document::doc_type::DocType;
pub use crate::import::model::Party;

/// ISDOC 6 namespace (also the version check: 5.x uses another one).
pub const NS: &str = "http://isdoc.cz/namespace/2013";
/// Namespace of `manifest.xml` in an `.isdocx`.
pub const MANIFEST_NS: &str = "http://isdoc.cz/namespace/2013/manifest";

/// An amount in the document currency, plus the plain (local, CZK) element
/// when the document is in a foreign currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Money {
    pub doc: Decimal,
    pub czk: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub description: String,
    pub quantity: Option<Decimal>,
    pub unit: Option<String>,
    /// `UnitPrice` (local currency).
    pub unit_price: Decimal,
    /// `LineExtensionAmount(Curr)`.
    pub base: Money,
    /// `LineExtensionAmountBeforeDiscount` (local currency).
    pub before_discount: Option<Decimal>,
    pub rate: Decimal,
}

/// One `TaxSubTotal`; amounts are the `Difference…` ones (what this
/// document taxes after taxed deposits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxRow {
    pub rate: Decimal,
    pub base: Money,
    pub vat: Money,
    pub vat_applicable: Option<bool>,
    pub reverse_charge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Totals {
    /// `DifferenceTaxExclusiveAmount`.
    pub base: Money,
    /// `DifferenceTaxInclusiveAmount`.
    pub total: Money,
    /// `TaxInclusiveAmount` (document currency), shown in the preview.
    pub gross: Decimal,
    /// `PayableRoundingAmount` (document currency), default 0.
    pub rounding: Decimal,
    pub payable: Money,
    /// `PaidDepositsAmount`: deducted non-taxed deposits.
    pub paid_deposits: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Payment {
    pub code: Option<String>,
    pub due_date: Option<NaiveDate>,
    /// Domestic account number (`ID`) and `BankCode`.
    pub account: Option<String>,
    pub bank_code: Option<String>,
    pub iban: Option<String>,
    pub bic: Option<String>,
    pub variable_symbol: Option<String>,
    pub constant_symbol: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub doc_type: DocType,
    pub number: String,
    pub issue_date: NaiveDate,
    pub tax_point_date: Option<NaiveDate>,
    /// Header `VATApplicable`: the supplier is a VAT payer.
    pub vat_applicable: bool,
    pub note: Option<String>,
    pub currency: String,
    /// `CurrRate / RefCurrRate` when usable (> 0, ≠ 1, local CZK).
    pub rate: Option<Decimal>,
    pub supplier: Party,
    pub customer: Option<Party>,
    pub original_ref: Option<String>,
    pub lines: Vec<Line>,
    pub recap: Vec<TaxRow>,
    pub totals: Totals,
    pub payment: Option<Payment>,
}

/// ISDOC `DocumentType` code ↔ document type.
pub fn doc_type_from_code(code: &str) -> Option<DocType> {
    Some(match code {
        "1" => DocType::Invoice,
        "2" => DocType::CreditNote,
        "3" => DocType::DebitNote,
        "4" => DocType::Proforma,
        "5" => DocType::AdvanceTaxDoc,
        "6" => DocType::AdvanceCreditNote,
        "7" => DocType::Simplified,
        _ => return None,
    })
}

pub fn doc_type_code(t: DocType) -> u8 {
    match t {
        DocType::CreditNote => 2,
        DocType::DebitNote => 3,
        DocType::Proforma => 4,
        DocType::AdvanceTaxDoc => 5,
        DocType::AdvanceCreditNote => 6,
        DocType::Simplified => 7,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for t in DocType::ALL.into_iter().filter(|t| t.is_document_type()) {
            assert_eq!(doc_type_from_code(&doc_type_code(t).to_string()), Some(t));
        }
        assert_eq!(doc_type_from_code("8"), None);
        assert_eq!(doc_type_from_code("0"), None);
    }
}
