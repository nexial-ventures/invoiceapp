//! The tax-date period shared by the accountant export and the overview
//! ([api/accounting.md](../../docs/api/accounting.md),
//! [api/overview.md](../../docs/api/overview.md)): its query check and the
//! document filter. Pure.

use chrono::NaiveDate;
use sea_orm::sea_query::{Expr, SimpleExpr};

use crate::error::FieldErrors;

/// Longest period, `to − from` in days.
pub const MAX_PERIOD_DAYS: i64 = 366;

/// The tax date of a document: DUZP, for a received one without it the
/// received date (issued documents have none).
pub const TAX_DATE: &str = "COALESCE(tax_point_date, received_date)";

/// `from` / `to` (`YYYY-MM-DD`, inclusive, `from ≤ to ≤ from + 366 days`);
/// errors (`invalid`) go to `e`, `None` then.
pub fn check(
    e: &mut FieldErrors,
    from: Option<&str>,
    to: Option<&str>,
) -> Option<(NaiveDate, NaiveDate)> {
    let date = |s: Option<&str>| {
        s.and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
            .ok_or("invalid")
    };
    let from = e.check("from", date(from));
    let to = e.check("to", date(to));
    let (from, to) = (from?, to?);
    if to < from || (to - from).num_days() > MAX_PERIOD_DAYS {
        e.add("from", "invalid");
        e.add("to", "invalid");
        return None;
    }
    Some((from, to))
}

/// Documents whose tax date is in `from..=to`.
pub fn tax_date_between(from: NaiveDate, to: NaiveDate) -> SimpleExpr {
    Expr::cust_with_values(format!("{TAX_DATE} BETWEEN $1 AND $2"), [from, to])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        from: Option<&str>,
        to: Option<&str>,
    ) -> (Option<(NaiveDate, NaiveDate)>, Vec<&'static str>) {
        let mut e = FieldErrors::new();
        let r = check(&mut e, from, to);
        let fields = ["from", "to"]
            .into_iter()
            .filter(|f| e.get(f).is_some())
            .collect();
        (r, fields)
    }

    #[test]
    fn periods() {
        let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("date");
        assert_eq!(
            run(Some(" 2026-01-01"), Some("2026-01-31")),
            (Some((date(2026, 1, 1), date(2026, 1, 31))), vec![])
        );
        assert_eq!(
            run(Some("2027-03-01"), Some("2028-03-01")).0,
            Some((date(2027, 3, 1), date(2028, 3, 1))),
            "366 days"
        );
        assert_eq!(
            run(Some("2026-01-01"), Some("2027-01-03")).1,
            ["from", "to"]
        );
        assert_eq!(
            run(Some("2026-02-01"), Some("2026-01-31")).1,
            ["from", "to"]
        );
        assert_eq!(run(None, Some("2026-01-31")).1, ["from"]);
        assert_eq!(run(Some("2026-01-01"), Some("31.01.2026")).1, ["to"]);
    }
}
