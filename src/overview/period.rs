//! The months of the overview chart. Pure.

use chrono::{Datelike, Days, NaiveDate};

/// Months of the selected period below which the chart shows the trailing
/// 12 months instead.
const MIN_MONTHS: i32 = 2;
const TRAILING: i32 = 12;

/// The chart's months and the tax-date range its bars cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chart {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// First day of each month, ascending.
    pub months: Vec<NaiveDate>,
}

fn index(d: NaiveDate) -> i32 {
    d.year() * 12 + d.month0() as i32
}

fn first_of(index: i32) -> Option<NaiveDate> {
    let (y, m0) = (index.div_euclid(12), index.rem_euclid(12));
    NaiveDate::from_ymd_opt(y, m0 as u32 + 1, 1)
}

fn last_of(index: i32) -> Option<NaiveDate> {
    first_of(index + 1)?.checked_sub_days(Days::new(1))
}

fn months(first: i32, last: i32) -> Option<Vec<NaiveDate>> {
    (first..=last).map(first_of).collect()
}

/// The months the period `from..=to` touches. Two or more → those months,
/// bars over the period itself; fewer → the trailing 12 whole months ending
/// with `to`'s month (a superset of the period). `None` at the ends of the
/// calendar.
pub fn chart(from: NaiveDate, to: NaiveDate) -> Option<Chart> {
    let (first, last) = (index(from), index(to));
    if last - first + 1 >= MIN_MONTHS {
        return Some(Chart {
            from,
            to,
            months: months(first, last)?,
        });
    }
    let first = last - TRAILING + 1;
    Some(Chart {
        from: first_of(first)?,
        to: last_of(last)?,
        months: months(first, last)?,
    })
}

/// `YYYY-MM` of a date.
pub fn month_key(d: NaiveDate) -> String {
    d.format("%Y-%m").to_string()
}

/// Whether the month starting `first` overlaps `from..=to`.
pub fn overlaps(first: NaiveDate, from: NaiveDate, to: NaiveDate) -> bool {
    first <= to && last_of(index(first)).is_none_or(|last| last >= from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("date")
    }

    fn keys(c: &Chart) -> Vec<String> {
        c.months.iter().copied().map(month_key).collect()
    }

    fn chart(from: NaiveDate, to: NaiveDate) -> Chart {
        super::chart(from, to).expect("chart")
    }

    #[test]
    fn calendar_ends() {
        assert_eq!(super::chart(NaiveDate::MAX, NaiveDate::MAX), None);
        assert_eq!(super::chart(NaiveDate::MIN, NaiveDate::MIN), None);
    }

    #[test]
    fn a_month_shows_the_trailing_year() {
        let c = chart(date(2026, 1, 1), date(2026, 1, 31));
        assert_eq!((c.from, c.to), (date(2025, 2, 1), date(2026, 1, 31)));
        assert_eq!(c.months.len(), 12);
        assert_eq!(keys(&c)[0], "2025-02");
        assert_eq!(keys(&c)[11], "2026-01");
        // A part of a month: still whole months.
        let c = chart(date(2024, 2, 10), date(2024, 2, 12));
        assert_eq!((c.from, c.to), (date(2023, 3, 1), date(2024, 2, 29)));
    }

    #[test]
    fn longer_periods_show_their_months() {
        let c = chart(date(2026, 1, 1), date(2026, 3, 31));
        assert_eq!(keys(&c), ["2026-01", "2026-02", "2026-03"]);
        assert_eq!((c.from, c.to), (date(2026, 1, 1), date(2026, 3, 31)));
        // Two touched months, across a year end, clipped to the period.
        let c = chart(date(2025, 12, 20), date(2026, 1, 5));
        assert_eq!(keys(&c), ["2025-12", "2026-01"]);
        assert_eq!((c.from, c.to), (date(2025, 12, 20), date(2026, 1, 5)));
        let year = chart(date(2026, 1, 1), date(2026, 12, 31));
        assert_eq!(year.months.len(), 12);
        let leap = chart(date(2027, 3, 1), date(2028, 3, 1));
        assert_eq!(leap.months.len(), 13);
    }

    #[test]
    fn overlap() {
        let (from, to) = (date(2026, 1, 15), date(2026, 2, 3));
        assert!(overlaps(date(2026, 1, 1), from, to));
        assert!(overlaps(date(2026, 2, 1), from, to));
        assert!(!overlaps(date(2025, 12, 1), from, to));
        assert!(!overlaps(date(2026, 3, 1), from, to));
    }
}
