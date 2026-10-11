//! The sample invoice behind `GET /api/pdf/preview` (never stored).

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;

use super::format::Locale;
use super::payload::Input;
use crate::document::compute::{self, Params};
use crate::document::handlers::dto::{BankSnapshot, Line, PartySnapshot, Totals, lines_out};
use crate::document::line::{ItemData, LineData, RoundingMode, Status, VatMode};
use crate::document::repo::issue::supplier;
use crate::error::AppError;
use crate::settings::entity::{bank_account, company};
use crate::time::today;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

pub struct Sample {
    locale: Locale,
    number: String,
    issue_date: NaiveDate,
    due_date: NaiveDate,
    supplier: PartySnapshot,
    customer: PartySnapshot,
    bank: Option<BankSnapshot>,
    lines: Vec<Line>,
    totals: Totals,
}

fn item(description: &str, qty: i64, unit: &str, price: i64, discount: i64, rate: i64) -> LineData {
    LineData::Item(ItemData {
        description: description.into(),
        quantity: Decimal::from(qty),
        unit: Some(unit.into()),
        unit_price: Decimal::from(price),
        discount_pct: Decimal::from(discount),
        vat_rate: Decimal::from(rate),
    })
}

fn sample_lines(locale: Locale) -> Vec<LineData> {
    let cs = locale == Locale::Cs;
    let t = |c: &'static str, e: &'static str| if cs { c } else { e };
    vec![
        item(t("Konzultace", "Consulting"), 8, "h", 1500, 0, 21),
        item(
            t("Vývoj webové aplikace", "Web application development"),
            24,
            "h",
            1200,
            10,
            21,
        ),
        item(
            t("Odborná publikace", "Technical book"),
            2,
            t("ks", "pcs"),
            450,
            0,
            12,
        ),
        item(
            t("Hosting – server", "Hosting – server"),
            12,
            t("měs.", "mo"),
            300,
            0,
            21,
        ),
        item(
            t("Hosting – zálohování", "Hosting – backups"),
            12,
            t("měs.", "mo"),
            100,
            0,
            21,
        ),
        LineData::Subtotal {
            description: t("Hosting na rok", "Annual hosting").into(),
            refs: vec![4, 5],
            collapse: true,
        },
        LineData::Text {
            description: t("Děkujeme za spolupráci.", "Thank you for your business.").into(),
        },
    ]
}

fn sample_customer(locale: Locale) -> PartySnapshot {
    let cs = locale == Locale::Cs;
    PartySnapshot {
        name: if cs {
            "Ukázkový odběratel s.r.o."
        } else {
            "Sample Customer Ltd."
        }
        .into(),
        ico: Some("12345679".into()),
        dic: Some("CZ12345679".into()),
        street: "Vzorová 12".into(),
        city: "Brno".into(),
        zip: "602 00".into(),
        country: "CZ".into(),
        registration: None,
        vat_payer: None,
        email: Some("fakturace@example.com".into()),
        phone: None,
        web: None,
    }
}

impl Sample {
    /// Today's sample for `company` with its default CZK account.
    pub async fn load(
        db: &DatabaseConnection,
        company: &company::Model,
        locale: Locale,
    ) -> Result<Sample, AppError> {
        let bank = Self::bank(db, company).await?;
        Ok(Sample::new(company, bank.as_ref(), locale, today())?)
    }

    /// The account the sample shows: the space's default CZK one.
    pub async fn bank(
        db: &DatabaseConnection,
        company: &company::Model,
    ) -> Result<Option<bank_account::Model>, AppError> {
        Ok(bank_account::Entity::find()
            .filter(bank_account::Column::SpaceId.eq(company.space_id))
            .filter(bank_account::Column::Currency.eq("CZK"))
            .filter(bank_account::Column::IsDefault.eq(true))
            .one(db)
            .await?)
    }

    pub fn new(
        company: &company::Model,
        bank: Option<&bank_account::Model>,
        locale: Locale,
        today: NaiveDate,
    ) -> anyhow::Result<Sample> {
        let data = sample_lines(locale);
        let evaluated = compute::evaluate(
            &data,
            Params {
                vat_mode: VatMode::Standard,
                is_czk: true,
                exchange_rate: None,
                rounding: RoundingMode::None,
            },
        )
        .map_err(|e| anyhow::anyhow!("sample invoice does not compute: {e:?}"))?;
        let mut supplier = supplier(company);
        if supplier.name.trim().is_empty() {
            supplier.name = match locale {
                Locale::Cs => "Vaše firma s.r.o.",
                Locale::En => "Your Company Ltd.",
            }
            .into();
        }
        Ok(Sample {
            locale,
            number: format!("{}0001", today.year()),
            issue_date: today,
            due_date: today + chrono::Days::new(14),
            supplier,
            customer: sample_customer(locale),
            bank: bank.map(|b| BankSnapshot {
                account_number: b.account_number.clone(),
                iban: b.iban.clone(),
                bic: b.bic.clone(),
            }),
            lines: lines_out(&data, &evaluated.lines),
            totals: evaluated.totals.into(),
        })
    }

    pub fn input(&self) -> Input<'_> {
        Input {
            locale: self.locale,
            status: Status::Issued,
            doc_type: "invoice",
            vat_mode: VatMode::Standard,
            number: Some(&self.number),
            issue_date: self.issue_date,
            tax_point_date: Some(self.issue_date),
            due_date: self.due_date,
            currency: "CZK",
            rate: None,
            payment_method: "bank_transfer",
            variable_symbol: Some(&self.number),
            constant_symbol: None,
            order_ref: None,
            header_note: None,
            footer_note: None,
            correction_reason: None,
            parent_number: None,
            parent_doc_type: None,
            supplier: Some(&self.supplier),
            customer: Some(&self.customer),
            bank: self.bank.as_ref(),
            lines: &self.lines,
            totals: &self.totals,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::payload;

    fn company() -> company::Model {
        company::Model {
            space_id: uuid::Uuid::nil(),
            name: String::new(),
            ico: None,
            dic: None,
            vat_payer: true,
            street: String::new(),
            city: String::new(),
            zip: String::new(),
            country: "CZ".into(),
            email: None,
            phone: None,
            web: None,
            registration: None,
            default_due_days: 14,
            default_locale: "cs".into(),
            default_rounding_cash: "unit".into(),
            default_rounding_other: "none".into(),
            default_payment_method: "bank_transfer".into(),
            default_currency: "CZK".into(),
            default_header_note_cs: None,
            default_header_note_en: None,
            default_footer_note_cs: None,
            default_footer_note_en: None,
            updated_at: chrono::Utc::now().into(),
        }
    }

    fn bank() -> bank_account::Model {
        bank_account::Model {
            id: uuid::Uuid::nil(),
            space_id: uuid::Uuid::nil(),
            label: None,
            currency: "CZK".into(),
            account_number: Some("19-2000145399/0800".into()),
            iban: Some("CZ6508000000192000145399".into()),
            bic: None,
            is_default: true,
            created_at: chrono::Utc::now().into(),
        }
    }

    #[test]
    fn sample_has_the_promised_shape() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 8).expect("date");
        let s = Sample::new(&company(), Some(&bank()), Locale::Cs, today).expect("sample");
        let p = payload::build(&s.input()).expect("payload");
        assert_eq!(p.number.as_deref(), Some("20260001"));
        assert_eq!(p.supplier.name, "Vaše firma s.r.o.");
        assert!(!p.draft);
        assert!(p.has_discount);
        assert!(p.qr.is_some());
        let kinds: Vec<&str> = p.lines.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, ["item", "item", "item", "subtotal", "text"]);
        let rates: Vec<&str> = p
            .vat_recap
            .as_ref()
            .expect("recap")
            .rows
            .iter()
            .map(|r| r.rate.as_str())
            .collect();
        assert_eq!(rates, ["21\u{a0}%", "12\u{a0}%"]);

        let en = Sample::new(&company(), None, Locale::En, today).expect("sample");
        let p = payload::build(&en.input()).expect("payload");
        assert_eq!(p.supplier.name, "Your Company Ltd.");
        assert!(p.qr.is_none());
        assert_eq!(p.locale, "en");
    }
}
