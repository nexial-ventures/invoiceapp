//! Every fixed text printed on the PDF, per locale — the template carries none.

use super::format::Locale;
use crate::document::doc_type::DocType;
use crate::document::line::PaymentMethod;
use crate::document::line::VatMode;

pub struct Labels {
    pub supplier: &'static str,
    pub customer: &'static str,
    pub ico: &'static str,
    pub dic: &'static str,
    pub issue_date: &'static str,
    pub tax_point_date: &'static str,
    pub due_date: &'static str,
    pub payment_date: &'static str,
    pub payment_method: &'static str,
    pub account_number: &'static str,
    pub iban: &'static str,
    pub bic: &'static str,
    pub variable_symbol: &'static str,
    pub constant_symbol: &'static str,
    pub order_ref: &'static str,
    pub description: &'static str,
    pub quantity: &'static str,
    pub unit_price: &'static str,
    pub discount: &'static str,
    pub vat_rate: &'static str,
    pub base: &'static str,
    /// The amount column of a non-payer (no VAT).
    pub amount: &'static str,
    pub vat: &'static str,
    pub total: &'static str,
    pub vat_recap: &'static str,
    pub vat_recap_czk: &'static str,
    pub total_excl_vat: &'static str,
    pub total_incl_vat: &'static str,
    pub advances: &'static str,
    pub rounding: &'static str,
    pub payable: &'static str,
    pub rate: &'static str,
    pub paid_note: &'static str,
    pub qr: &'static str,
    pub watermark: &'static str,
    pub cancelled: &'static str,
    pub page: &'static str,
    pub issued_by: &'static str,
}

const CS: Labels = Labels {
    supplier: "Dodavatel",
    customer: "Odběratel",
    ico: "IČO",
    dic: "DIČ",
    issue_date: "Datum vystavení",
    tax_point_date: "Datum zdanitelného plnění",
    due_date: "Datum splatnosti",
    payment_date: "Datum přijetí platby",
    payment_method: "Způsob platby",
    account_number: "Číslo účtu",
    iban: "IBAN",
    bic: "BIC/SWIFT",
    variable_symbol: "Variabilní symbol",
    constant_symbol: "Konstantní symbol",
    order_ref: "Objednávka",
    description: "Popis",
    quantity: "Množství",
    unit_price: "Cena za MJ",
    discount: "Sleva",
    vat_rate: "DPH",
    base: "Základ",
    amount: "Celkem",
    vat: "DPH",
    total: "Celkem",
    vat_recap: "Rekapitulace DPH",
    vat_recap_czk: "Rekapitulace DPH v CZK",
    total_excl_vat: "Celkem bez DPH",
    total_incl_vat: "Celkem s DPH",
    advances: "Odpočet záloh",
    rounding: "Zaokrouhlení",
    payable: "K úhradě",
    rate: "Sazba",
    paid_note: "Neplaťte – již uhrazeno.",
    qr: "QR platba",
    watermark: "NÁVRH",
    cancelled: "STORNO",
    page: "Strana",
    issued_by: "Vystavil",
};

const EN: Labels = Labels {
    supplier: "Supplier",
    customer: "Customer",
    ico: "Company ID",
    dic: "VAT ID",
    issue_date: "Issue date",
    tax_point_date: "Tax point date",
    due_date: "Due date",
    payment_date: "Payment received",
    payment_method: "Payment method",
    account_number: "Account number",
    iban: "IBAN",
    bic: "BIC/SWIFT",
    variable_symbol: "Variable symbol",
    constant_symbol: "Constant symbol",
    order_ref: "Order reference",
    description: "Description",
    quantity: "Quantity",
    unit_price: "Unit price",
    discount: "Discount",
    vat_rate: "VAT",
    base: "Net",
    amount: "Amount",
    vat: "VAT",
    total: "Total",
    vat_recap: "VAT summary",
    vat_recap_czk: "VAT summary in CZK",
    total_excl_vat: "Total excl. VAT",
    total_incl_vat: "Total incl. VAT",
    advances: "Advances deducted",
    rounding: "Rounding",
    payable: "Amount due",
    rate: "Rate",
    paid_note: "Do not pay – already paid.",
    qr: "QR payment",
    watermark: "DRAFT",
    cancelled: "CANCELLED",
    page: "Page",
    issued_by: "Issued by",
};

pub fn labels(locale: Locale) -> &'static Labels {
    match locale {
        Locale::Cs => &CS,
        Locale::En => &EN,
    }
}

pub fn title(doc_type: &str, vat_mode: VatMode, locale: Locale) -> &'static str {
    let cs = locale == Locale::Cs;
    match DocType::parse(doc_type) {
        Some(DocType::Proforma) if cs => "Zálohová faktura",
        Some(DocType::Proforma) => "Proforma invoice",
        Some(DocType::CreditNote) if cs => "Opravný daňový doklad",
        Some(DocType::CreditNote) => "Credit note",
        Some(DocType::DebitNote) if cs => "Opravný daňový doklad – vrubopis",
        Some(DocType::DebitNote) => "Debit note",
        Some(DocType::AdvanceTaxDoc) if cs => "Daňový doklad k přijaté platbě",
        Some(DocType::AdvanceTaxDoc) => "Tax document for a received payment",
        Some(DocType::AdvanceCreditNote) if cs => "Opravný daňový doklad k přijaté platbě",
        Some(DocType::AdvanceCreditNote) => "Advance payment correction",
        Some(DocType::Simplified) if cs => "Zjednodušený daňový doklad",
        Some(DocType::Simplified) => "Simplified tax document",
        _ if vat_mode == VatMode::NonPayer && cs => "Faktura",
        _ if vat_mode == VatMode::NonPayer => "Invoice",
        _ if cs => "Faktura – daňový doklad",
        _ => "Invoice – tax document",
    }
}

pub fn legal_note(vat_mode: VatMode, locale: Locale) -> Option<&'static str> {
    let cs = locale == Locale::Cs;
    Some(match vat_mode {
        VatMode::Standard => return None,
        VatMode::ReverseCharge if cs => {
            "Daň odvede zákazník (přenesení daňové povinnosti, § 92a zákona o DPH)."
        }
        VatMode::ReverseCharge => "Reverse charge – VAT to be accounted for by the customer.",
        VatMode::Exempt if cs => "Plnění osvobozené od DPH.",
        VatMode::Exempt => "VAT exempt supply.",
        VatMode::NonPayer if cs => "Dodavatel není plátcem DPH.",
        VatMode::NonPayer => "The supplier is not a VAT payer.",
    })
}

pub fn payment_method(method: &str, locale: Locale) -> &'static str {
    let cs = locale == Locale::Cs;
    match PaymentMethod::parse(method) {
        Some(PaymentMethod::BankTransfer) if cs => "Bankovní převod",
        Some(PaymentMethod::BankTransfer) => "Bank transfer",
        Some(PaymentMethod::Cash) if cs => "Hotově",
        Some(PaymentMethod::Cash) => "Cash",
        Some(PaymentMethod::Card) if cs => "Kartou",
        Some(PaymentMethod::Card) => "Card",
        _ if cs => "Jiný",
        _ => "Other",
    }
}

/// Country name for an ISO code; unknown codes print as the code.
pub fn country(code: &str, locale: Locale) -> String {
    const NAMES: [(&str, &str, &str); 16] = [
        ("CZ", "Česká republika", "Czech Republic"),
        ("SK", "Slovensko", "Slovakia"),
        ("DE", "Německo", "Germany"),
        ("AT", "Rakousko", "Austria"),
        ("PL", "Polsko", "Poland"),
        ("HU", "Maďarsko", "Hungary"),
        ("GB", "Spojené království", "United Kingdom"),
        ("US", "Spojené státy", "United States"),
        ("FR", "Francie", "France"),
        ("IT", "Itálie", "Italy"),
        ("NL", "Nizozemsko", "Netherlands"),
        ("BE", "Belgie", "Belgium"),
        ("ES", "Španělsko", "Spain"),
        ("CH", "Švýcarsko", "Switzerland"),
        ("IE", "Irsko", "Ireland"),
        ("SE", "Švédsko", "Sweden"),
    ];
    NAMES
        .iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, cs, en)| match locale {
            Locale::Cs => (*cs).to_string(),
            Locale::En => (*en).to_string(),
        })
        .unwrap_or_else(|| code.to_string())
}

/// `Kurz ČNB 24,400 CZK/EUR ze dne 7. 10. 2026`; without a ČNB date
/// (manual rate) just `Kurz 24,400 CZK/EUR`.
pub fn rate_note(rate: &str, currency: &str, cnb_date: Option<&str>, locale: Locale) -> String {
    match (locale, cnb_date) {
        (Locale::Cs, Some(d)) => format!("Kurz ČNB {rate} CZK/{currency} ze dne {d}"),
        (Locale::Cs, None) => format!("Kurz {rate} CZK/{currency}"),
        (Locale::En, Some(d)) => format!("ČNB rate {rate} CZK/{currency} of {d}"),
        (Locale::En, None) => format!("Rate {rate} CZK/{currency}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_follow_the_contract() {
        let cs = Locale::Cs;
        assert_eq!(
            title("invoice", VatMode::Standard, cs),
            "Faktura – daňový doklad"
        );
        assert_eq!(
            title("invoice", VatMode::ReverseCharge, cs),
            "Faktura – daňový doklad"
        );
        assert_eq!(title("invoice", VatMode::NonPayer, cs), "Faktura");
        assert_eq!(title("invoice", VatMode::NonPayer, Locale::En), "Invoice");
        assert_eq!(title("proforma", VatMode::NonPayer, cs), "Zálohová faktura");
        assert_eq!(
            title("credit_note", VatMode::Standard, Locale::En),
            "Credit note"
        );
        assert_eq!(
            title("advance_tax_doc", VatMode::Standard, cs),
            "Daňový doklad k přijaté platbě"
        );
        assert_eq!(
            title("invoice", VatMode::Exempt, Locale::En),
            "Invoice – tax document"
        );
        assert_eq!(
            title("debit_note", VatMode::Standard, cs),
            "Opravný daňový doklad – vrubopis"
        );
        assert_eq!(
            title("advance_credit_note", VatMode::Standard, Locale::En),
            "Advance payment correction"
        );
        assert_eq!(
            title("simplified", VatMode::NonPayer, cs),
            "Zjednodušený daňový doklad"
        );
    }

    #[test]
    fn legal_notes_by_vat_mode() {
        assert_eq!(legal_note(VatMode::Standard, Locale::Cs), None);
        assert_eq!(
            legal_note(VatMode::Exempt, Locale::Cs),
            Some("Plnění osvobozené od DPH.")
        );
        assert_eq!(
            legal_note(VatMode::NonPayer, Locale::En),
            Some("The supplier is not a VAT payer.")
        );
        assert!(
            legal_note(VatMode::ReverseCharge, Locale::Cs).is_some_and(|t| t.contains("§ 92a"))
        );
    }

    #[test]
    fn rate_notes() {
        assert_eq!(
            rate_note("24,400", "EUR", Some("7. 10. 2026"), Locale::Cs),
            "Kurz ČNB 24,400 CZK/EUR ze dne 7. 10. 2026"
        );
        assert_eq!(
            rate_note("24.400", "EUR", None, Locale::En),
            "Rate 24.400 CZK/EUR"
        );
    }

    #[test]
    fn countries_and_methods() {
        assert_eq!(country("SK", Locale::Cs), "Slovensko");
        assert_eq!(country("DE", Locale::En), "Germany");
        assert_eq!(country("JP", Locale::Cs), "JP");
        assert_eq!(payment_method("cash", Locale::Cs), "Hotově");
        assert_eq!(payment_method("other", Locale::En), "Other");
        assert_eq!(labels(Locale::En).watermark, "DRAFT");
    }
}
