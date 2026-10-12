//! Rows → CSV as the export writes it: `;`, UTF-8 with BOM, CRLF, RFC 4180
//! quoting, decimal comma, `dd.mm.yyyy`. Pure. Used by the sample file and
//! the export (2c).

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::format::{
    AFTER_RATES, BEFORE_RATES, base_column, format_amount, format_bool, format_date, format_rate,
    guard_text, vat_column,
};
use crate::document::doc_type::DocType;
use crate::document::line::{Direction, VatMode};
use crate::import::model::Party;

/// One document, amounts as stored (positive; [`OutRow::cells`] negates a
/// credit note's), the recap in CZK.
#[derive(Debug, Clone, PartialEq)]
pub struct OutRow {
    pub direction: Direction,
    pub doc_type: DocType,
    pub number: Option<String>,
    pub supplier_number: Option<String>,
    pub related_number: Option<String>,
    pub issue_date: NaiveDate,
    pub tax_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub received_date: Option<NaiveDate>,
    pub counterparty: Option<Party>,
    pub currency: String,
    pub exchange_rate: Option<Decimal>,
    pub vat_mode: VatMode,
    /// `(rate, base CZK, VAT CZK)`.
    pub recap: Vec<(Decimal, Decimal, Decimal)>,
    pub rounding: Decimal,
    pub total: Decimal,
    pub total_czk: Option<Decimal>,
    pub paid_date: Option<NaiveDate>,
    pub variable_symbol: Option<String>,
    pub vat_deductible: Option<bool>,
    pub category: Option<String>,
    pub note: Option<String>,
}

/// The rate columns of `rates`: highest first, `base_{r}` + `vat_{r}`, only
/// `base_0` for rate 0.
pub fn rate_columns(rates: &[Decimal]) -> Vec<(Decimal, String, Option<String>)> {
    let mut rates: Vec<Decimal> = rates.iter().map(|r| r.normalize()).collect();
    rates.sort_by_key(|r| std::cmp::Reverse(*r));
    rates.dedup();
    rates
        .into_iter()
        .map(|r| (r, base_column(r), (!r.is_zero()).then(|| vat_column(r))))
        .collect()
}

pub fn header(rates: &[Decimal]) -> Vec<String> {
    let mut h: Vec<String> = BEFORE_RATES.iter().map(|c| c.to_string()).collect();
    for (_, base, vat) in rate_columns(rates) {
        h.push(base);
        h.extend(vat);
    }
    h.extend(AFTER_RATES.iter().map(|c| c.to_string()));
    h
}

fn opt<T>(x: Option<T>, fmt: impl Fn(T) -> String) -> String {
    x.map(fmt).unwrap_or_default()
}

impl OutRow {
    /// The cells under [`header`]`(rates)`; a recap rate missing from
    /// `rates` is not written.
    pub fn cells(&self, rates: &[Decimal]) -> Vec<String> {
        let k = Decimal::from(self.doc_type.sign());
        let amount = |x: Decimal| format_amount(x * k);
        let party = self.counterparty.as_ref();
        let pf = |g: fn(&Party) -> String| guard_text(&opt(party, g));
        let text = |x: &Option<String>| guard_text(x.as_deref().unwrap_or_default());
        let mut c = vec![
            self.direction.as_str().to_string(),
            self.doc_type.as_str().to_string(),
            text(&self.number),
            text(&self.supplier_number),
            text(&self.related_number),
            format_date(self.issue_date),
            opt(self.tax_date, format_date),
            opt(self.due_date, format_date),
            opt(self.received_date, format_date),
            pf(|p| p.name.clone()),
            pf(|p| p.ico.clone().unwrap_or_default()),
            pf(|p| p.dic.clone().unwrap_or_default()),
            pf(|p| p.street.clone()),
            pf(|p| p.city.clone()),
            pf(|p| p.zip.clone()),
            pf(|p| p.country.clone()),
            self.currency.clone(),
            opt(self.exchange_rate, format_rate),
            self.vat_mode.as_str().to_string(),
        ];
        for (rate, _, vat) in rate_columns(rates) {
            let row = self.recap.iter().find(|r| r.0.normalize() == rate);
            c.push(opt(row, |r| amount(r.1)));
            if vat.is_some() {
                c.push(opt(row, |r| amount(r.2)));
            }
        }
        c.extend([
            amount(self.rounding),
            amount(self.total),
            opt(self.total_czk, amount),
            opt(self.paid_date, format_date),
            text(&self.variable_symbol),
            opt(self.vat_deductible, |b| format_bool(b).to_string()),
            text(&self.category),
            text(&self.note),
        ]);
        c
    }
}

fn records(
    start: Vec<u8>,
    records: impl IntoIterator<Item = Vec<String>>,
) -> anyhow::Result<Vec<u8>> {
    let mut w = csv::WriterBuilder::new()
        .delimiter(b';')
        .terminator(csv::Terminator::CRLF)
        .from_writer(start);
    for r in records {
        w.write_record(r)?;
    }
    w.into_inner().map_err(|e| anyhow::anyhow!("{e}"))
}

/// The start of the file: BOM and header.
pub fn head(rates: &[Decimal]) -> anyhow::Result<Vec<u8>> {
    records(b"\xEF\xBB\xBF".to_vec(), [header(rates)])
}

/// Rows only, to follow [`head`] (the export streams them in chunks).
pub fn body(rates: &[Decimal], rows: &[OutRow]) -> anyhow::Result<Vec<u8>> {
    records(Vec::new(), rows.iter().map(|r| r.cells(rates)))
}

/// The whole file: BOM, header, rows.
pub fn write(rates: &[Decimal], rows: &[OutRow]) -> anyhow::Result<Vec<u8>> {
    let mut out = head(rates)?;
    out.extend(body(rates, rows)?);
    Ok(out)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap_or_default()
}

fn party(name: &str, ico: &str, city: &str) -> Party {
    Party {
        name: name.into(),
        ico: Some(ico.into()),
        dic: Some(format!("CZ{ico}")),
        street: "Vzorová 1".into(),
        city: city.into(),
        zip: "11000".into(),
        country: "CZ".into(),
        ..Default::default()
    }
}

/// The sample rows: an issued CZK invoice, a received EUR invoice and a
/// credit note of the first (fictitious parties).
pub fn sample_rows() -> Vec<OutRow> {
    let d = |s: &str| s.parse::<Decimal>().unwrap_or_default();
    let invoice = OutRow {
        direction: Direction::Issued,
        doc_type: DocType::Invoice,
        number: Some("2026000001".into()),
        supplier_number: None,
        related_number: None,
        issue_date: date(2026, 1, 15),
        tax_date: Some(date(2026, 1, 15)),
        due_date: Some(date(2026, 1, 29)),
        received_date: None,
        counterparty: Some(party("Fiktivní Odběratel s.r.o.", "12345679", "Praha")),
        currency: "CZK".into(),
        exchange_rate: None,
        vat_mode: VatMode::Standard,
        recap: vec![(d("21"), d("10000"), d("2100"))],
        rounding: Decimal::ZERO,
        total: d("12100"),
        total_czk: Some(d("12100")),
        paid_date: Some(date(2026, 1, 27)),
        variable_symbol: Some("2026000001".into()),
        vat_deductible: None,
        category: Some("Služby".into()),
        note: Some("Vzorový řádek".into()),
    };
    let received = OutRow {
        direction: Direction::Received,
        doc_type: DocType::Invoice,
        number: None,
        supplier_number: Some("FV-2026-0042".into()),
        issue_date: date(2026, 1, 20),
        tax_date: Some(date(2026, 1, 20)),
        due_date: Some(date(2026, 2, 3)),
        received_date: Some(date(2026, 1, 22)),
        counterparty: Some(party("Vzorový Dodavatel a.s.", "87654326", "Brno")),
        currency: "EUR".into(),
        exchange_rate: Some(d("24.335")),
        recap: vec![(d("21"), d("2433.50"), d("511.04"))],
        total: d("121"),
        total_czk: Some(d("2944.54")),
        paid_date: None,
        variable_symbol: Some("20260042".into()),
        vat_deductible: Some(true),
        category: Some("Software".into()),
        note: None,
        ..invoice.clone()
    };
    let credit = OutRow {
        doc_type: DocType::CreditNote,
        number: Some("2026000002".into()),
        related_number: Some("2026000001".into()),
        issue_date: date(2026, 2, 2),
        tax_date: Some(date(2026, 2, 2)),
        due_date: Some(date(2026, 2, 16)),
        recap: vec![(d("21"), d("1000"), d("210"))],
        total: d("1210"),
        total_czk: Some(d("1210")),
        paid_date: None,
        variable_symbol: Some("2026000002".into()),
        note: None,
        ..invoice.clone()
    };
    vec![invoice, received, credit]
}

/// The sample file over `rates` (Settings → VAT rates) plus the rates the
/// sample rows use.
pub fn sample(rates: &[Decimal]) -> anyhow::Result<Vec<u8>> {
    let rows = sample_rows();
    let mut all = rates.to_vec();
    all.extend(rows.iter().flat_map(|r| r.recap.iter().map(|x| x.0)));
    write(&all, &rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csvio::format::rate_column;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    #[test]
    fn header_rate_columns() {
        let h = header(&[d("0"), d("21.00"), d("12"), d("12.5"), d("21")]);
        let rates: Vec<&str> = h
            .iter()
            .filter(|c| matches!(rate_column(c), Some(Ok(_))))
            .map(String::as_str)
            .collect();
        assert_eq!(
            rates,
            [
                "base_21",
                "vat_21",
                "base_12_5",
                "vat_12_5",
                "base_12",
                "vat_12",
                "base_0"
            ]
        );
        assert_eq!(h[0], crate::csvio::format::DIRECTION);
        assert_eq!(
            h.last().map(String::as_str),
            Some(crate::csvio::format::NOTE)
        );
    }

    #[test]
    fn sample_file_bytes() {
        let bytes = sample(&[d("12"), d("0")]).expect("sample");
        let text = String::from_utf8(bytes).expect("utf-8");
        assert!(text.starts_with('\u{feff}'));
        let lines: Vec<&str> = text.trim_start_matches('\u{feff}').split("\r\n").collect();
        assert_eq!(lines.len(), 5, "header + 3 rows + trailing CRLF");
        assert!(lines[0].contains(";base_21;vat_21;base_12;vat_12;base_0;rounding;"));
        assert!(lines[1].starts_with("issued;invoice;2026000001;;;15.01.2026;"));
        assert!(lines[1].contains(";10000,00;2100,00;;;;0,00;12100,00;12100,00;27.01.2026;"));
        assert!(lines[2].contains(";EUR;24,335;standard;2433,50;511,04;"));
        assert!(lines[2].contains(";1;Software;"));
        assert!(
            lines[3].contains(";-1000,00;-210,00;;;;0,00;-1210,00;-1210,00;"),
            "{}",
            lines[3]
        );
    }

    #[test]
    fn quoting() {
        let mut r = sample_rows().remove(0);
        r.note = Some("a; \"b\"\nc".into());
        let bytes = write(&[d("21")], &[r]).expect("write");
        let text = String::from_utf8(bytes).expect("utf-8");
        assert!(text.contains(";\"a; \"\"b\"\"\nc\"\r\n"), "{text}");
    }

    #[test]
    fn formula_cells_are_guarded() {
        let mut r = sample_rows().remove(2);
        r.note = Some("=HYPERLINK(\"x\")".into());
        r.category = Some("@cat".into());
        r.counterparty = r.counterparty.map(|p| Party {
            name: "+420 Firma".into(),
            city: "'-x".into(),
            ..p
        });
        let bytes = write(&[d("21")], &[r]).expect("write");
        let text = String::from_utf8(bytes).expect("utf-8");
        let row = text.split("\r\n").nth(1).expect("row");
        assert!(row.contains(";'+420 Firma;"), "{row}");
        assert!(row.contains(";''-x;"), "{row}");
        assert!(row.contains(";'@cat;\"'=HYPERLINK(\"\"x\"\")\""), "{row}");
        assert!(
            row.contains(";-1000,00;-210,00;"),
            "amounts stay numbers: {row}"
        );
    }
}
