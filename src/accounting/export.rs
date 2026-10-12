//! The XML files of the accountant export (Pohoda, Money S3): every
//! document mapped inside the snapshot before anything is sent, so the
//! accountant never gets a file with documents silently missing.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use sea_orm::{ConnectionTrait, DatabaseTransaction};
use uuid::Uuid;

use super::doc::shown_number;
use super::pohoda_summary::third_rate;
use super::settings::{AccountingSettings, Program};
use super::{money, pohoda};
use crate::error::AppError;
use crate::export::load::{CHUNK, Loaded, rollback};
use crate::export::source::Source;
use crate::settings::repo::{company, vat_rates};
use crate::space::SpaceId;
use crate::time::today;

/// Longest `detail` of an `unexportable` error, in characters.
const MAX_DETAIL: usize = 2000;

/// The file the accountant export writes (`format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The CSV of the import format.
    Csv,
    Xml(Program),
}

impl Format {
    /// `csv` (also when absent) | `pohoda` | `money`; anything else → `None`.
    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s.map(str::trim) {
            None | Some("csv") => Some(Format::Csv),
            Some("pohoda") => Some(Format::Xml(Program::Pohoda)),
            Some("money") => Some(Format::Xml(Program::Money)),
            Some(_) => None,
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Format::Csv => "text/csv; charset=utf-8",
            Format::Xml(Program::Pohoda) => "application/xml; charset=windows-1250",
            Format::Xml(Program::Money) => "application/xml; charset=utf-8",
        }
    }
}

/// What every item of one export shares.
pub struct Ctx {
    /// The agenda IČO (Pohoda `dataPack/@ico`, Money `@ICAgendy`): the
    /// program's settings override, else the company IČO.
    pub ico: Option<String>,
    pub settings: AccountingSettings,
    /// Pohoda's `price3` rate: the first active Settings rate other than
    /// 0 / 12 / 21 (loaded for Pohoda only; Money has no such slot).
    pub third: Option<Decimal>,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

fn label(program: Program) -> &'static str {
    match program {
        Program::Pohoda => "accountant pohoda",
        Program::Money => "accountant money",
    }
}

async fn ctx<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    program: Program,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Ctx, AppError> {
    let settings = super::repo::get(db, space).await?;
    let company = company::get(db, space).await?;
    let third = match program {
        Program::Pohoda => {
            let rates: Vec<(Decimal, bool)> = vat_rates::list(db, space)
                .await?
                .into_iter()
                .map(|r| (r.rate, r.active))
                .collect();
            third_rate(&rates)
        }
        Program::Money => None,
    };
    Ok(Ctx {
        ico: settings.section(program).ico.clone().or(company.ico),
        settings,
        third,
        from,
        to,
    })
}

/// 422 `documents: unexportable`, `detail` = `"<number>: <reason>; …"`
/// cut to [`MAX_DETAIL`] with the count of the documents left out.
pub fn unexportable(failures: &[(String, String)]) -> AppError {
    let mut detail = String::new();
    for (i, (doc, reason)) in failures.iter().enumerate() {
        let part = format!("{}{doc}: {reason}", if i == 0 { "" } else { "; " });
        if detail.chars().count() + part.chars().count() > MAX_DETAIL {
            detail.push_str(&format!("; … (+{} more)", failures.len() - i));
            break;
        }
        detail.push_str(&part);
    }
    AppError::field_detail("documents", "unexportable", detail)
}

/// The file being written: Pohoda appends to one buffer, Money collects
/// per list ([`money::Lists`]).
enum Out {
    Pohoda(String),
    Money(money::Lists),
}

impl Out {
    fn new(program: Program, ctx: &Ctx) -> Self {
        match program {
            Program::Pohoda => Out::Pohoda(pohoda::head(ctx)),
            Program::Money => Out::Money(money::Lists::default()),
        }
    }

    fn push(&mut self, s: &Source, ctx: &Ctx) -> anyhow::Result<()> {
        match self {
            Out::Pohoda(out) => out.push_str(&pohoda::item(s, ctx)?),
            Out::Money(lists) => {
                let (list, item) = money::item(s, ctx)?;
                lists.push(list, &item);
            }
        }
        Ok(())
    }

    fn finish(self, ctx: &Ctx) -> Vec<u8> {
        match self {
            Out::Pohoda(mut out) => {
                out.push_str(pohoda::TAIL);
                pohoda::encode(&out)
            }
            Out::Money(lists) => lists.finish(ctx, today()).into_bytes(),
        }
    }
}

/// Every item, then the encoded file.
async fn build(
    txn: &DatabaseTransaction,
    ids: &[Uuid],
    ctx: &Ctx,
    program: Program,
) -> Result<Vec<u8>, AppError> {
    let mut out = Out::new(program, ctx);
    let mut failures = Vec::new();
    let none = Default::default();
    for chunk in ids.chunks(CHUNK) {
        let loaded = Loaded::load(txn, chunk, false).await?;
        for s in loaded.sources(&none) {
            if let Err(e) = out.push(&s, ctx) {
                let name = shown_number(s.doc).map_or_else(|| s.doc.id.to_string(), str::to_string);
                tracing::warn!(export = label(program), document = %s.doc.id, error = %format!("{e:#}"), "document not exportable");
                failures.push((name, format!("{e:#}")));
            }
        }
    }
    if !failures.is_empty() {
        return Err(unexportable(&failures));
    }
    Ok(out.finish(ctx))
}

/// The `program` file of `ids` (already ordered), read in `txn` (ended
/// here). No document → 422 `from: empty` (an empty Pohoda `dataPack` is
/// not valid, and an empty file helps nobody); one that cannot be mapped →
/// [`unexportable`].
pub async fn file(
    txn: DatabaseTransaction,
    space: SpaceId,
    ids: &[Uuid],
    program: Program,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<u8>, AppError> {
    if ids.is_empty() {
        rollback(txn, label(program)).await;
        return Err(AppError::field("from", "empty"));
    }
    let built = match ctx(&txn, space, program, from, to).await {
        Ok(c) => build(&txn, ids, &c, program).await,
        Err(e) => Err(e),
    };
    match built {
        Ok(bytes) => {
            txn.commit().await?;
            Ok(bytes)
        }
        Err(e) => {
            rollback(txn, label(program)).await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(e: AppError) -> (Option<&'static str>, String) {
        match e {
            AppError::ValidationDetail(f, d) => (f.get("documents"), d),
            other => panic!("expected validation detail, got {other:?}"),
        }
    }

    #[test]
    fn formats() {
        assert_eq!(Format::parse(None), Some(Format::Csv));
        assert_eq!(
            Format::parse(Some(" money ")),
            Some(Format::Xml(Program::Money))
        );
        assert_eq!(Format::parse(Some("xlsx")), None);
        assert_eq!(
            Format::Xml(Program::Pohoda).content_type(),
            "application/xml; charset=windows-1250"
        );
        assert_eq!(Format::Csv.content_type(), "text/csv; charset=utf-8");
    }

    #[test]
    fn unexportable_detail() {
        let f = |n: &str| {
            (
                n.to_string(),
                "VAT rate 10 % maps to no Pohoda rate slot".to_string(),
            )
        };
        let (reason, d) = detail(unexportable(&[f("FV-50"), f("FA-9")]));
        assert_eq!(reason, Some("unexportable"));
        assert_eq!(
            d,
            "FV-50: VAT rate 10 % maps to no Pohoda rate slot; FA-9: VAT rate 10 % maps to no Pohoda rate slot"
        );
        let many: Vec<_> = (0..200).map(|i| f(&format!("N-{i}"))).collect();
        let (_, d) = detail(unexportable(&many));
        assert!(d.chars().count() <= MAX_DETAIL + 20, "{}", d.len());
        assert!(d.starts_with("N-0: ") && d.ends_with("more)"), "{d}");
    }
}
