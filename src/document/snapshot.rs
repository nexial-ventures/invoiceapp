//! Value types frozen on a document (party / bank snapshots) and its totals
//! as served: shared by the API, PDF, ISDOC, accounting exports and e-mail.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::document::compute::{self, RecapRow};

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Recap {
    pub vat_rate: Decimal,
    pub base: Decimal,
    pub vat: Decimal,
    pub base_czk: Option<Decimal>,
    pub vat_czk: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    /// Sorted by rate, highest first.
    pub recap: Vec<Recap>,
    pub base: Decimal,
    pub vat: Decimal,
    pub total: Decimal,
    pub rounding: Decimal,
    pub payable: Decimal,
    pub total_czk: Option<Decimal>,
}

impl From<compute::Totals> for Totals {
    fn from(t: compute::Totals) -> Self {
        Self {
            recap: t
                .recap
                .into_iter()
                .map(|r: RecapRow| Recap {
                    vat_rate: r.vat_rate.normalize(),
                    base: r.base,
                    vat: r.vat,
                    base_czk: r.base_czk,
                    vat_czk: r.vat_czk,
                })
                .collect(),
            base: t.base,
            vat: t.vat,
            total: t.total,
            rounding: t.rounding,
            payable: t.payable,
            total_czk: t.total_czk,
        }
    }
}

/// Party data frozen at issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PartySnapshot {
    pub name: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub street: String,
    pub city: String,
    pub zip: String,
    pub country: String,
    pub registration: Option<String>,
    pub vat_payer: Option<bool>,
    /// Contact lines printed on the PDF (absent in pre-1d snapshots).
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub web: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BankSnapshot {
    pub account_number: Option<String>,
    pub iban: Option<String>,
    pub bic: Option<String>,
}
