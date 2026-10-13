//! HTTP client for the ARES REST API and the mapping of its JSON to [`AresSubject`].

use std::time::Duration;

use anyhow::{Context, anyhow};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::AppError;

pub const DEFAULT_ARES_URL: &str = "https://ares.gov.cz/ekonomicke-subjekty-v-be/rest";
const TIMEOUT: Duration = Duration::from_secs(10);

/// A contact draft built from an ARES record (no id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AresSubject {
    pub name: String,
    pub ico: String,
    pub dic: Option<String>,
    pub street: String,
    pub city: String,
    pub zip: String,
    pub country: String,
}

#[derive(Clone)]
pub struct AresClient {
    http: reqwest::Client,
    base_url: String,
}

impl AresClient {
    /// `base_url` is the REST root, e.g. [`DEFAULT_ARES_URL`] (trailing `/` ignored).
    pub fn new(base_url: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("invoiceapp/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("build ARES HTTP client")?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Look up a validated IČO. ARES 404 → [`AppError::AresNotFound`]; network
    /// failure, timeout, any other non-2xx or an unparsable body →
    /// [`AppError::AresUnavailable`].
    pub async fn lookup(&self, ico: &str) -> Result<AresSubject, AppError> {
        let url = format!("{}/ekonomicke-subjekty/{ico}", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .with_context(|| format!("GET {url}"))
            .map_err(AppError::AresUnavailable)?;
        match resp.status() {
            StatusCode::NOT_FOUND => return Err(AppError::AresNotFound),
            s if !s.is_success() => {
                return Err(AppError::AresUnavailable(anyhow!("GET {url}: HTTP {s}")));
            }
            _ => {}
        }
        let body = resp
            .bytes()
            .await
            .with_context(|| format!("read body of GET {url}"))
            .map_err(AppError::AresUnavailable)?;
        parse_subject(&body, ico).map_err(AppError::AresUnavailable)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSubject {
    ico: Option<String>,
    obchodni_jmeno: Option<String>,
    dic: Option<String>,
    #[serde(default)]
    sidlo: RawAddress,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAddress {
    nazev_ulice: Option<String>,
    nazev_obce: Option<String>,
    nazev_casti_obce: Option<String>,
    cislo_domovni: Option<NumOrText>,
    cislo_orientacni: Option<NumOrText>,
    cislo_orientacni_pismeno: Option<String>,
    psc: Option<NumOrText>,
    textova_adresa: Option<String>,
}

/// ARES sends numbers as JSON numbers; accept strings too to be lenient.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NumOrText {
    Num(u64),
    Text(String),
}

impl NumOrText {
    fn text(&self) -> String {
        match self {
            Self::Num(n) => n.to_string(),
            Self::Text(s) => s.trim().to_string(),
        }
    }
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Parse an ARES `ekonomicke-subjekty/{ico}` body. `requested_ico` fills in a
/// missing `ico`; a record without `obchodniJmeno` is treated as malformed.
pub fn parse_subject(body: &[u8], requested_ico: &str) -> anyhow::Result<AresSubject> {
    let raw: RawSubject = serde_json::from_slice(body).context("parse ARES JSON")?;
    let name = non_empty(&raw.obchodni_jmeno)
        .context("ARES record has no obchodniJmeno")?
        .to_string();
    let a = &raw.sidlo;
    let city = non_empty(&a.nazev_obce).unwrap_or_default().to_string();
    Ok(AresSubject {
        name,
        ico: non_empty(&raw.ico).unwrap_or(requested_ico).to_string(),
        dic: non_empty(&raw.dic).map(str::to_string),
        street: street(a),
        city,
        zip: a.psc.as_ref().map(zip).unwrap_or_default(),
        country: "CZ".to_string(),
    })
}

/// `Ulice čp/čo[písmeno]`; without a street the part of the municipality (or
/// the municipality) stands in for it; with no structured data at all, the
/// free-text `textovaAdresa`.
fn street(a: &RawAddress) -> String {
    let house = a.cislo_domovni.as_ref().map(NumOrText::text);
    let orient = a.cislo_orientacni.as_ref().map(|n| {
        format!(
            "{}{}",
            n.text(),
            non_empty(&a.cislo_orientacni_pismeno).unwrap_or_default()
        )
    });
    let number = match (house, orient) {
        (Some(h), Some(o)) => format!("{h}/{o}"),
        (Some(h), None) => h,
        (None, Some(o)) => o,
        (None, None) => String::new(),
    };
    let name = non_empty(&a.nazev_ulice).or_else(|| {
        (!number.is_empty())
            .then(|| non_empty(&a.nazev_casti_obce).or_else(|| non_empty(&a.nazev_obce)))
            .flatten()
    });
    let street = [name.unwrap_or_default(), number.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    if street.is_empty() {
        non_empty(&a.textova_adresa).unwrap_or_default().to_string()
    } else {
        street
    }
}

/// PSČ as a 5-digit string (ARES sends a number, so leading zeros are lost).
fn zip(psc: &NumOrText) -> String {
    match psc {
        NumOrText::Num(n) => format!("{n:05}"),
        NumOrText::Text(s) => s.chars().filter(|c| !c.is_whitespace()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: serde_json::Value) -> AresSubject {
        parse_subject(json.to_string().as_bytes(), "27074358").expect("parse")
    }

    #[test]
    fn maps_full_address_with_orientation_number() {
        let s = parse(serde_json::json!({
            "ico": "27074358",
            "obchodniJmeno": "Example a.s.",
            "dic": "CZ27074358",
            "sidlo": {
                "nazevObce": "Praha", "nazevCastiObce": "Michle", "nazevUlice": "Budějovická",
                "cisloDomovni": 778, "cisloOrientacni": 3, "cisloOrientacniPismeno": "a",
                "psc": 14000, "textovaAdresa": "Budějovická 778/3a, Michle, 14000 Praha 4"
            }
        }));
        assert_eq!(
            s,
            AresSubject {
                name: "Example a.s.".into(),
                ico: "27074358".into(),
                dic: Some("CZ27074358".into()),
                street: "Budějovická 778/3a".into(),
                city: "Praha".into(),
                zip: "14000".into(),
                country: "CZ".into(),
            }
        );
    }

    #[test]
    fn village_without_street_uses_part_of_municipality() {
        let s = parse(serde_json::json!({
            "obchodniJmeno": "Jan Novák",
            "sidlo": { "nazevObce": "Lhota", "nazevCastiObce": "Dolní Lhota", "cisloDomovni": 12, "psc": 1234 }
        }));
        assert_eq!(s.street, "Dolní Lhota 12");
        assert_eq!(s.city, "Lhota");
        assert_eq!(s.zip, "01234");
        assert_eq!(s.ico, "27074358");
        assert_eq!(s.dic, None);
    }

    #[test]
    fn falls_back_to_municipality_then_text_address() {
        let s = parse(serde_json::json!({
            "obchodniJmeno": "X",
            "sidlo": { "nazevObce": "Lhota", "cisloDomovni": "5" }
        }));
        assert_eq!(s.street, "Lhota 5");
        let s = parse(serde_json::json!({
            "obchodniJmeno": "X",
            "sidlo": { "textovaAdresa": "Někde 1, 11000 Praha" }
        }));
        assert_eq!(s.street, "Někde 1, 11000 Praha");
        let s = parse(serde_json::json!({ "obchodniJmeno": "X" }));
        assert_eq!(
            (s.street.as_str(), s.city.as_str(), s.zip.as_str()),
            ("", "", "")
        );
    }

    #[test]
    fn street_without_house_number() {
        let s = parse(serde_json::json!({
            "obchodniJmeno": "X",
            "sidlo": { "nazevUlice": "Dlouhá", "cisloOrientacni": 7, "psc": "110 00" }
        }));
        assert_eq!(s.street, "Dlouhá 7");
        assert_eq!(s.zip, "11000");
    }

    #[test]
    fn rejects_garbage_and_nameless_records() {
        assert!(parse_subject(b"<html>oops</html>", "27074358").is_err());
        assert!(parse_subject(b"{\"ico\":\"27074358\"}", "27074358").is_err());
    }
}
