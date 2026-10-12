//! HTTP client for the ČNB daily rate list (`denni_kurz.txt`) and its parser.

use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};

pub const DEFAULT_CNB_URL: &str = "https://www.cnb.cz/cs/financni-trhy/devizovy-trh/kurzy-devizoveho-trhu/kurzy-devizoveho-trhu/denni_kurz.txt";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Matches the `numeric(18,6)` rate columns.
pub const RATE_DP: u32 = 6;

/// One published list: its publication date and CZK per 1 unit of each currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateList {
    pub published: NaiveDate,
    pub rates: Vec<(String, Decimal)>,
}

impl RateList {
    pub fn get(&self, currency: &str) -> Option<Decimal> {
        self.rates
            .iter()
            .find(|(c, _)| c == currency)
            .map(|(_, r)| *r)
    }
}

#[derive(Clone)]
pub struct CnbClient {
    http: reqwest::Client,
    url: String,
    /// Lists not final yet, reused for [`super::recent::TTL`].
    pub recent: super::recent::Recent,
}

impl CnbClient {
    /// `url` is the full `denni_kurz.txt` URL, e.g. [`DEFAULT_CNB_URL`].
    pub fn new(url: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("invoice/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("build ČNB HTTP client")?;
        Ok(Self {
            http,
            url: url.to_string(),
            recent: Default::default(),
        })
    }

    /// The list ČNB publishes for `date` (the latest on or before it).
    pub async fn fetch(&self, date: NaiveDate) -> anyhow::Result<RateList> {
        let url = format!("{}?date={}", self.url, date.format("%d.%m.%Y"));
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            bail!("GET {url}: HTTP {status}");
        }
        let body = resp
            .text()
            .await
            .with_context(|| format!("read body of GET {url}"))?;
        parse_rates(&body)
    }
}

/// Parse `denni_kurz.txt`: `08.10.2026 #196`, a header line, then
/// `země|měna|množství|kód|kurz` rows with comma decimals; rate = kurz / množství.
pub fn parse_rates(text: &str) -> anyhow::Result<RateList> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().context("empty ČNB response")?;
    let date = first.split_whitespace().next().unwrap_or_default();
    let published = NaiveDate::parse_from_str(date, "%d.%m.%Y")
        .with_context(|| format!("ČNB header without a date: {first:?}"))?;
    lines
        .next()
        .context("ČNB response without a column header")?;
    let rates = lines.map(parse_row).collect::<anyhow::Result<Vec<_>>>()?;
    if rates.is_empty() {
        bail!("ČNB response without rates");
    }
    Ok(RateList { published, rates })
}

fn parse_row(row: &str) -> anyhow::Result<(String, Decimal)> {
    let cols: Vec<&str> = row.split('|').map(str::trim).collect();
    let [_, _, amount, code, rate] = cols[..] else {
        return Err(anyhow!("malformed ČNB row: {row:?}"));
    };
    let amount: Decimal = amount
        .parse::<u32>()
        .ok()
        .filter(|a| *a > 0)
        .map(Decimal::from)
        .with_context(|| format!("bad amount in ČNB row: {row:?}"))?;
    let rate: Decimal = rate
        .replace(',', ".")
        .parse()
        .with_context(|| format!("bad rate in ČNB row: {row:?}"))?;
    if rate <= Decimal::ZERO || code.len() != 3 {
        bail!("bad ČNB row: {row:?}");
    }
    let per_unit = (rate / amount)
        .round_dp_with_strategy(RATE_DP, RoundingStrategy::MidpointAwayFromZero)
        .normalize();
    Ok((code.to_ascii_uppercase(), per_unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "08.10.2026 #196\n\
        země|měna|množství|kód|kurz\n\
        Austrálie|dolar|1|AUD|15,234\n\
        EMU|euro|1|EUR|24,335\n\
        Japonsko|jen|100|JPY|15,512\n\
        Maďarsko|forint|100|HUF|6,543\n\
        Indonesie|rupie|1000|IDR|1,447\n";

    fn dec(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    #[test]
    fn parses_comma_decimals_and_amounts() {
        let list = parse_rates(SAMPLE).expect("parse");
        assert_eq!(
            list.published,
            NaiveDate::from_ymd_opt(2026, 10, 8).expect("date")
        );
        assert_eq!(list.get("EUR"), Some(dec("24.335")));
        assert_eq!(list.get("JPY"), Some(dec("0.15512")));
        assert_eq!(list.get("HUF"), Some(dec("0.06543")));
        assert_eq!(list.get("IDR"), Some(dec("0.001447")));
        assert_eq!(list.rates.len(), 5);
    }

    #[test]
    fn missing_currency_is_none() {
        let list = parse_rates(SAMPLE).expect("parse");
        assert_eq!(list.get("XYZ"), None);
    }

    #[test]
    fn tolerates_crlf_and_trailing_blank_lines() {
        let text = SAMPLE.replace('\n', "\r\n") + "\r\n\r\n";
        assert_eq!(parse_rates(&text).expect("parse").rates.len(), 5);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_rates("").is_err());
        assert!(parse_rates("<html>maintenance</html>").is_err());
        assert!(parse_rates("08.10.2026 #196\nheader\n").is_err());
        assert!(parse_rates("08.10.2026 #196\nheader\nEMU|euro|1|EUR\n").is_err());
        assert!(parse_rates("08.10.2026 #196\nheader\nEMU|euro|0|EUR|24,3\n").is_err());
        assert!(parse_rates("08.10.2026 #196\nheader\nEMU|euro|1|EUR|abc\n").is_err());
    }
}
