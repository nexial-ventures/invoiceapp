//! The template context: display values pre-formatted in the template's
//! locale exactly like the PDF payload; missing values are `none`.

use rust_decimal::Decimal;
use serde::Serialize;

use crate::document::doc_type::DocType;
use crate::document::line::Status;
use crate::document::state::NO_PAYMENTS_DOC_TYPE;
use crate::pdf::format::{self, Locale};
use crate::pdf::labels;
use crate::pdf::payload::Input;
use crate::settings::entity::company;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    #[serde(rename = "type")]
    pub doc_type: String,
    pub type_label: String,
    pub number: Option<String>,
    pub issue_date: String,
    pub tax_date: Option<String>,
    pub due_date: Option<String>,
    pub total: String,
    pub payable: String,
    pub currency: String,
    pub paid: bool,
    pub cancelled: bool,
    /// Not paid, not cancelled and owed to us: the reminder line applies.
    pub to_pay: bool,
    pub variable_symbol: Option<String>,
    pub bank_account: Option<String>,
    pub iban: Option<String>,
    pub original_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Company {
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub web: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Contact {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Context {
    pub doc: Doc,
    pub company: Company,
    pub contact: Option<Contact>,
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Company {
    pub fn from_row(c: &company::Model) -> Self {
        Self {
            name: c.name.clone(),
            email: non_empty(c.email.as_deref()),
            phone: non_empty(c.phone.as_deref()),
            web: non_empty(c.web.as_deref()),
        }
    }
}

/// `paid` = the sum of payments; `contact_email` = the live contact's
/// address, which wins over the issue-time snapshot.
pub fn build(
    i: &Input,
    locale: Locale,
    paid: Decimal,
    company: Company,
    contact_email: Option<&str>,
) -> Context {
    let sign = Decimal::from(DocType::parse_document(i.doc_type).map_or(1, |t| t.sign()));
    let money = |x: Decimal| format::money(x * sign, i.currency, locale);
    let remaining = (i.totals.payable - paid).max(Decimal::ZERO);
    let settled = i.doc_type == NO_PAYMENTS_DOC_TYPE.as_str() || remaining.is_zero();
    let cancelled = i.status == Status::Cancelled;
    let correction = DocType::parse_document(i.doc_type).is_some_and(|t| t.is_correction());
    let bank = i.bank;
    Context {
        doc: Doc {
            doc_type: i.doc_type.to_string(),
            type_label: labels::title(i.doc_type, i.vat_mode, locale).to_string(),
            number: i.number.map(str::to_string),
            issue_date: format::date(i.issue_date, locale),
            tax_date: i.tax_point_date.map(|d| format::date(d, locale)),
            due_date: Some(format::date(i.due_date, locale)),
            total: money(i.totals.total),
            payable: money(remaining),
            currency: i.currency.to_string(),
            paid: settled,
            cancelled,
            to_pay: !settled && !cancelled && sign > Decimal::ZERO,
            variable_symbol: non_empty(i.variable_symbol),
            bank_account: non_empty(bank.and_then(|b| b.account_number.as_deref())),
            iban: non_empty(bank.and_then(|b| b.iban.as_deref())).map(|s| format::iban(&s)),
            original_number: i.parent_number.filter(|_| correction).map(str::to_string),
        },
        company,
        contact: i.customer.map(|c| Contact {
            name: c.name.clone(),
            email: non_empty(contact_email).or_else(|| non_empty(c.email.as_deref())),
        }),
    }
}

/// The sample variants a template is validated on, so every
/// branch-relevant variable takes each of its shapes: `(prefix, context)`.
/// The first (empty prefix) is the base sample previews render; the prefixes
/// lead the `template_invalid` detail of a failure on that variant.
pub fn samples(base: Input, locale: Locale, company: &Company) -> Vec<(&'static str, Context)> {
    let ctx = |i: &Input, paid| build(i, locale, paid, company.clone(), None);
    let mut cancelled = base;
    cancelled.status = Status::Cancelled;
    let mut credit = base;
    credit.doc_type = "credit_note";
    credit.parent_doc_type = Some("invoice");
    credit.parent_number = base.number;
    let mut bare = base;
    bare.bank = None;
    bare.variable_symbol = None;
    bare.tax_point_date = None;
    let mut bare = ctx(&bare, Decimal::ZERO);
    bare.doc.due_date = None;
    let mut contactless = base;
    contactless.doc_type = "simplified";
    contactless.customer = None;
    vec![
        ("", ctx(&base, Decimal::ZERO)),
        ("paid", ctx(&base, base.totals.payable)),
        ("cancelled", ctx(&cancelled, Decimal::ZERO)),
        ("credit note", ctx(&credit, Decimal::ZERO)),
        ("no bank account", bare),
        ("without contact", ctx(&contactless, Decimal::ZERO)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::line::VatMode;
    use crate::document::snapshot::{BankSnapshot, PartySnapshot, Totals};
    use chrono::NaiveDate;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    fn totals(total: &str, payable: &str) -> Totals {
        Totals {
            recap: vec![],
            base: Decimal::ZERO,
            vat: Decimal::ZERO,
            total: d(total),
            rounding: Decimal::ZERO,
            payable: d(payable),
            total_czk: None,
        }
    }

    fn party(email: Option<&str>) -> PartySnapshot {
        PartySnapshot {
            name: "Odběratel a.s.".into(),
            ico: None,
            dic: None,
            street: String::new(),
            city: String::new(),
            zip: String::new(),
            country: "CZ".into(),
            registration: None,
            vat_payer: None,
            email: email.map(str::to_string),
            phone: None,
            web: None,
        }
    }

    fn input<'a>(
        doc_type: &'a str,
        status: Status,
        totals: &'a Totals,
        customer: Option<&'a PartySnapshot>,
        bank: Option<&'a BankSnapshot>,
    ) -> Input<'a> {
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).expect("date");
        Input {
            locale: Locale::Cs,
            status,
            doc_type,
            vat_mode: VatMode::Standard,
            number: Some("20260001"),
            issue_date: date,
            tax_point_date: Some(date),
            due_date: date + chrono::Days::new(14),
            currency: "CZK",
            rate: None,
            payment_method: "bank_transfer",
            variable_symbol: Some("20260001"),
            constant_symbol: None,
            order_ref: None,
            header_note: None,
            footer_note: None,
            correction_reason: None,
            parent_number: Some("20250099"),
            parent_doc_type: Some("invoice"),
            supplier: None,
            customer,
            bank,
            lines: &[],
            totals,
        }
    }

    fn company() -> Company {
        Company {
            name: "Dodavatel s.r.o.".into(),
            email: Some("firma@example.com".into()),
            phone: None,
            web: None,
        }
    }

    #[test]
    fn unpaid_invoice_in_both_locales() {
        let t = totals("1234.5", "1234.5");
        let customer = party(Some("old@example.com"));
        let bank = BankSnapshot {
            account_number: Some("19-2000145399/0800".into()),
            iban: Some("CZ6508000000192000145399".into()),
            bic: None,
        };
        let i = input("invoice", Status::Issued, &t, Some(&customer), Some(&bank));
        let c = build(
            &i,
            Locale::Cs,
            d("234.5"),
            company(),
            Some("new@example.com"),
        );
        assert_eq!(c.doc.type_label, "Faktura – daňový doklad");
        assert_eq!(c.doc.total, "1\u{a0}234,50\u{a0}Kč");
        assert_eq!(c.doc.payable, "1\u{a0}000,00\u{a0}Kč");
        assert_eq!(c.doc.issue_date, "1. 10. 2026");
        assert_eq!(c.doc.due_date.as_deref(), Some("15. 10. 2026"));
        assert!(!c.doc.paid && c.doc.to_pay && !c.doc.cancelled);
        assert_eq!(c.doc.iban.as_deref(), Some("CZ65 0800 0000 1920 0014 5399"));
        assert_eq!(c.doc.original_number, None);
        let contact = c.contact.expect("contact");
        assert_eq!(contact.email.as_deref(), Some("new@example.com"));

        let en = build(&i, Locale::En, Decimal::ZERO, company(), None);
        assert_eq!(en.doc.type_label, "Invoice – tax document");
        assert_eq!(en.doc.total, "CZK\u{a0}1,234.50");
        assert_eq!(en.doc.issue_date, "1 Oct 2026");
        assert_eq!(
            en.contact.and_then(|c| c.email).as_deref(),
            Some("old@example.com")
        );
    }

    #[tokio::test]
    async fn sample_variants_cover_every_branch() {
        let t = totals("1210", "1210");
        let customer = party(Some("a@example.com"));
        let bank = BankSnapshot {
            account_number: Some("19-2000145399/0800".into()),
            iban: Some("CZ6508000000192000145399".into()),
            bic: None,
        };
        let mut base = input("invoice", Status::Issued, &t, Some(&customer), Some(&bank));
        base.parent_number = None;
        let s = samples(base, Locale::Cs, &company());
        let names: Vec<&str> = s.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            [
                "",
                "paid",
                "cancelled",
                "credit note",
                "no bank account",
                "without contact"
            ]
        );
        let c = |i: usize| &s[i].1;
        assert!(c(0).doc.to_pay && c(0).doc.bank_account.is_some() && c(0).contact.is_some());
        assert!(c(1).doc.paid && !c(1).doc.to_pay);
        assert!(c(2).doc.cancelled && !c(2).doc.to_pay);
        assert_eq!(c(3).doc.original_number.as_deref(), Some("20260001"));
        assert!(c(3).doc.total.starts_with('-'));
        let bare = &c(4).doc;
        assert_eq!(
            (
                &bare.bank_account,
                &bare.iban,
                &bare.variable_symbol,
                &bare.due_date,
                &bare.tax_date
            ),
            (&None, &None, &None, &None, &None)
        );
        assert_eq!(
            (c(5).contact.as_ref(), c(5).doc.doc_type.as_str()),
            (None, "simplified")
        );

        // The default templates pass every variant.
        let values: Vec<(&str, minijinja::Value)> = s
            .iter()
            .map(|(n, c)| (*n, minijinja::Value::from_serialize(c)))
            .collect();
        for locale in [Locale::Cs, Locale::En] {
            let t = crate::email::templates::default(locale);
            crate::email::templates::validate(&t, &values)
                .await
                .expect("default template is valid");
        }
    }

    #[test]
    fn settled_cancelled_and_corrections() {
        let t = totals("1000", "1000");
        let paid = build(
            &input("invoice", Status::Issued, &t, None, None),
            Locale::Cs,
            d("1000"),
            company(),
            None,
        );
        assert!(paid.doc.paid && !paid.doc.to_pay);
        assert_eq!(paid.doc.payable, "0,00\u{a0}Kč");
        assert_eq!(paid.contact, None);

        let ddpp = build(
            &input("advance_tax_doc", Status::Issued, &t, None, None),
            Locale::Cs,
            Decimal::ZERO,
            company(),
            None,
        );
        assert!(ddpp.doc.paid);

        let cancelled = build(
            &input("invoice", Status::Cancelled, &t, None, None),
            Locale::Cs,
            Decimal::ZERO,
            company(),
            None,
        );
        assert!(cancelled.doc.cancelled && !cancelled.doc.to_pay);

        let credit = build(
            &input("credit_note", Status::Issued, &t, None, None),
            Locale::Cs,
            Decimal::ZERO,
            company(),
            None,
        );
        assert_eq!(credit.doc.total, "-1\u{a0}000,00\u{a0}Kč");
        assert!(!credit.doc.to_pay);
        assert_eq!(credit.doc.original_number.as_deref(), Some("20250099"));
    }
}
