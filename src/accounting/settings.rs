//! Settings → Accounting: per program, per direction × exported document
//! type, the optional codes the export writes. Pure (shape + validation).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::document::doc_type::DocType;
use crate::document::line::Direction;
use crate::error::{AppError, FieldErrors};
use crate::validation as v;

/// The document types the accounting exports hold (no proforma).
pub const EXPORTED: [DocType; 6] = [
    DocType::Invoice,
    DocType::CreditNote,
    DocType::DebitNote,
    DocType::AdvanceTaxDoc,
    DocType::AdvanceCreditNote,
    DocType::Simplified,
];

/// An accounting program the export writes for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    Pohoda,
    Money,
}

impl Program {
    /// The settings section and the prefix of its 422 keys.
    pub fn key(self) -> &'static str {
        match self {
            Program::Pohoda => "pohoda",
            Program::Money => "money",
        }
    }

    /// Longest code: Pohoda's `typ:idsType` (`maxLength` 19), Money's
    /// `zkratkaType` (10).
    pub fn code_max(self) -> usize {
        match self {
            Program::Pohoda => 19,
            Program::Money => 10,
        }
    }

    /// Longest number series: Pohoda `typ:ids` (19), Money `Rada` (5).
    pub fn series_max(self) -> usize {
        match self {
            Program::Pohoda => 19,
            Program::Money => 5,
        }
    }
}

/// The stored settings and both responses: every section, every row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AccountingSettings {
    #[serde(default)]
    pub pohoda: ProgramSettings,
    #[serde(default)]
    pub money: ProgramSettings,
}

/// Both responses: the settings plus the length limits `PUT` enforces, so
/// the form need not copy them.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct AccountingResponse {
    #[serde(flatten)]
    pub settings: AccountingSettings,
    pub limits: AccountingLimits,
}

/// Longest code per program, in characters after trimming.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, ToSchema)]
pub struct AccountingLimits {
    pub pohoda: CodeLimits,
    pub money: CodeLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeLimits {
    /// `accounting`, `classificationVat`, `classificationVatNonDeductible`.
    pub code: usize,
    pub number_series: usize,
}

impl CodeLimits {
    fn of(program: Program) -> Self {
        Self {
            code: program.code_max(),
            number_series: program.series_max(),
        }
    }
}

impl From<AccountingSettings> for AccountingResponse {
    fn from(settings: AccountingSettings) -> Self {
        Self {
            settings,
            limits: AccountingLimits {
                pohoda: CodeLimits::of(Program::Pohoda),
                money: CodeLimits::of(Program::Money),
            },
        }
    }
}

/// The `PUT` body: a section present replaces that section (`{}` clears
/// it), an absent (or null) one keeps what is stored.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, ToSchema)]
pub struct AccountingUpdate {
    #[serde(default)]
    pub pohoda: Option<ProgramSettings>,
    #[serde(default)]
    pub money: Option<ProgramSettings>,
}

/// One program's section (the same shape for Pohoda and Money S3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ProgramSettings {
    /// The agenda IČO (Pohoda `dataPack/@ico`, Money `MoneyData/@ICAgendy`);
    /// null → the company IČO.
    #[serde(default)]
    pub ico: Option<String>,
    /// Every direction × type row (GET); PUT may send any subset.
    #[serde(default)]
    pub codes: Vec<CodeRow>,
}

/// The codes of one direction × document type; null = not written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeRow {
    /// `issued` | `received`.
    pub direction: String,
    /// `invoice` | `credit_note` | `debit_note` | `advance_tax_doc` |
    /// `advance_credit_note` | `simplified`.
    pub doc_type: String,
    /// Předkontace (Pohoda `accounting/ids`, Money `PredKontac`).
    #[serde(default)]
    pub accounting: Option<String>,
    /// Členění DPH (Pohoda `classificationVAT/ids`, Money `KodDPH`).
    #[serde(default)]
    pub classification_vat: Option<String>,
    /// Received only: členění DPH of a document without the VAT deduction,
    /// never replaced by the code above (null → Pohoda `nonSubsume`, Money
    /// no `KodDPH`).
    #[serde(default)]
    pub classification_vat_non_deductible: Option<String>,
    /// Číselná řada (Pohoda `number/ids`, Money `Rada`).
    #[serde(default)]
    pub number_series: Option<String>,
}

impl CodeRow {
    fn empty(direction: Direction, doc_type: DocType) -> Self {
        Self {
            direction: direction.as_str().into(),
            doc_type: doc_type.as_str().into(),
            accounting: None,
            classification_vat: None,
            classification_vat_non_deductible: None,
            number_series: None,
        }
    }
}

fn exported(s: &str) -> Option<DocType> {
    DocType::parse_document(s).filter(|d| EXPORTED.contains(d))
}

impl ProgramSettings {
    /// Normalized (trim, `""` → null) and [`Self::full`]; every problem is
    /// recorded in `e` under `{program}.…`.
    fn validate(&self, program: Program, e: &mut FieldErrors) -> Self {
        let key = program.key();
        let code = |s: &Option<String>| v::opt_text(s.as_deref(), program.code_max());
        let ico = e
            .check(&format!("{key}.ico"), v::opt_ico(self.ico.as_deref()))
            .flatten();
        let mut codes: Vec<CodeRow> = Vec::new();
        for (i, r) in self.codes.iter().enumerate() {
            let f = |name: &str| format!("{key}.codes.{i}.{name}");
            let direction = Direction::parse(r.direction.trim());
            let direction = e.check(&f("direction"), direction.ok_or("invalid"));
            let doc_type = e.check(&f("docType"), exported(r.doc_type.trim()).ok_or("invalid"));
            let accounting = e.check(&f("accounting"), code(&r.accounting));
            let classification_vat = e.check(&f("classificationVat"), code(&r.classification_vat));
            let non_deductible = e.check(
                &f("classificationVatNonDeductible"),
                code(&r.classification_vat_non_deductible).and_then(|c| {
                    match (c.is_some(), direction) {
                        (true, Some(Direction::Issued)) => Err("invalid"),
                        _ => Ok(c),
                    }
                }),
            );
            let number_series = e.check(
                &f("numberSeries"),
                v::opt_text(r.number_series.as_deref(), program.series_max()),
            );
            let (Some(direction), Some(doc_type)) = (direction, doc_type) else {
                continue;
            };
            if codes
                .iter()
                .any(|c| c.direction == direction.as_str() && c.doc_type == doc_type.as_str())
            {
                e.add(&f("docType"), "duplicate");
                continue;
            }
            codes.push(CodeRow {
                accounting: accounting.flatten(),
                classification_vat: classification_vat.flatten(),
                classification_vat_non_deductible: non_deductible.flatten(),
                number_series: number_series.flatten(),
                ..CodeRow::empty(direction, doc_type)
            });
        }
        Self { ico, codes }.full()
    }

    /// Every direction × type row, in order (issued first, [`EXPORTED`]
    /// order); missing rows all null, unknown stored rows dropped.
    pub fn full(self) -> Self {
        let codes = Direction::ALL
            .iter()
            .copied()
            .flat_map(|d| EXPORTED.into_iter().map(move |t| (d, t)))
            .map(|(d, t)| {
                self.row(d, t)
                    .cloned()
                    .unwrap_or_else(|| CodeRow::empty(d, t))
            })
            .collect();
        Self {
            ico: self.ico,
            codes,
        }
    }

    /// The codes of `direction` × `doc_type`.
    pub fn row(&self, direction: Direction, doc_type: DocType) -> Option<&CodeRow> {
        self.codes
            .iter()
            .find(|c| c.direction == direction.as_str() && c.doc_type == doc_type.as_str())
    }
}

impl AccountingSettings {
    /// Every section with every row ([`ProgramSettings::full`]).
    pub fn full(self) -> Self {
        Self {
            pohoda: self.pohoda.full(),
            money: self.money.full(),
        }
    }

    pub fn section(&self, program: Program) -> &ProgramSettings {
        match program {
            Program::Pohoda => &self.pohoda,
            Program::Money => &self.money,
        }
    }
}

impl AccountingUpdate {
    /// Every present section normalized and full; the errors of both
    /// sections are reported together.
    pub fn validate(self) -> Result<Self, AppError> {
        let mut e = FieldErrors::new();
        let pohoda = self.pohoda.map(|s| s.validate(Program::Pohoda, &mut e));
        let money = self.money.map(|s| s.validate(Program::Money, &mut e));
        e.into_result()?;
        Ok(Self { pohoda, money })
    }

    /// `stored` with the present sections replaced.
    pub fn apply(self, stored: AccountingSettings) -> AccountingSettings {
        AccountingSettings {
            pohoda: self.pohoda.unwrap_or(stored.pohoda),
            money: self.money.unwrap_or(stored.money),
        }
        .full()
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
