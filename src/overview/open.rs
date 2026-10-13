//! Receivables and payables as of today: what is left to pay on issued
//! documents, per direction, in CZK. Pure.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::dto::{Open, OpenDoc};
use crate::document::doc_type::DocType;
use crate::document::line::Direction;

/// Overdue documents listed per direction.
pub const TOP: usize = 5;

/// An issued-status document with `paid < payable` (never a DDPP; a proforma
/// only while no invoice settles it).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRow {
    pub id: Uuid,
    pub direction: Direction,
    pub doc_type: DocType,
    pub number: Option<String>,
    pub counterparty: Option<String>,
    pub due_date: Option<NaiveDate>,
    /// `payable − paid` in CZK, positive as stored.
    pub remaining: Decimal,
}

impl OpenRow {
    fn overdue(&self, today: NaiveDate) -> bool {
        self.due_date.is_some_and(|d| d < today)
    }
}

/// The open documents of `direction`: tax documents (signed: an open credit
/// note reduces) with the longest overdue first, proformas as advances.
pub fn open(rows: &[OpenRow], direction: Direction, today: NaiveDate) -> Open {
    let mut o = Open::default();
    let mut overdue: Vec<&OpenRow> = Vec::new();
    for r in rows.iter().filter(|r| r.direction == direction) {
        let amount = r.remaining * Decimal::from(r.doc_type.sign());
        if r.doc_type == DocType::Proforma {
            o.advances.count += 1;
            o.advances.amount += amount;
            continue;
        }
        o.count += 1;
        o.amount += amount;
        if r.overdue(today) {
            o.overdue_count += 1;
            o.overdue_amount += amount;
            overdue.push(r);
        }
    }
    overdue.sort_by(|a, b| a.due_date.cmp(&b.due_date).then(a.number.cmp(&b.number)));
    o.top_overdue = overdue
        .into_iter()
        .take(TOP)
        .filter_map(|r| {
            Some(OpenDoc {
                id: r.id,
                doc_type: r.doc_type,
                number: r.number.clone(),
                counterparty: r.counterparty.clone(),
                due_date: r.due_date?,
                remaining: r.remaining * Decimal::from(r.doc_type.sign()),
            })
        })
        .collect();
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).expect("date")
    }

    fn row(n: u8, direction: Direction, doc_type: DocType, due: u32, remaining: &str) -> OpenRow {
        OpenRow {
            id: Uuid::from_u128(n.into()),
            direction,
            doc_type,
            number: Some(format!("D-{n}")),
            counterparty: Some("Firma".into()),
            due_date: Some(date(due)),
            remaining: d(remaining),
        }
    }

    #[test]
    fn receivables_split_overdue_and_advances() {
        let today = date(13);
        let rows = [
            row(1, Direction::Issued, DocType::Invoice, 20, "1000"),
            row(2, Direction::Issued, DocType::Invoice, 5, "500.50"),
            row(3, Direction::Issued, DocType::CreditNote, 1, "100"),
            row(4, Direction::Issued, DocType::Proforma, 2, "300"),
            row(5, Direction::Received, DocType::Invoice, 1, "999"),
            // Due today is not overdue yet.
            row(6, Direction::Issued, DocType::Simplified, 13, "10"),
        ];
        let o = open(&rows, Direction::Issued, today);
        assert_eq!((o.count, o.amount), (4, d("1410.50")));
        assert_eq!((o.overdue_count, o.overdue_amount), (2, d("400.50")));
        assert_eq!((o.advances.count, o.advances.amount), (1, d("300")));
        let top: Vec<_> = o
            .top_overdue
            .iter()
            .map(|t| (t.number.as_deref(), t.remaining))
            .collect();
        assert_eq!(top, [(Some("D-3"), d("-100")), (Some("D-2"), d("500.50"))]);

        let p = open(&rows, Direction::Received, today);
        assert_eq!((p.count, p.overdue_count, p.amount), (1, 1, d("999")));
        assert_eq!(p.top_overdue[0].id, Uuid::from_u128(5));
    }

    #[test]
    fn top_is_capped_and_ordered() {
        let rows: Vec<_> = (1..=7)
            .rev()
            .map(|n| row(n, Direction::Received, DocType::Invoice, n.into(), "1"))
            .collect();
        let o = open(&rows, Direction::Received, date(30));
        assert_eq!(o.overdue_count, 7);
        let days: Vec<_> = o.top_overdue.iter().map(|t| t.due_date).collect();
        assert_eq!(days, (1..=5).map(date).collect::<Vec<_>>());
        let mut no_due = row(9, Direction::Received, DocType::Invoice, 1, "5");
        no_due.due_date = None;
        let o = open(&[no_due], Direction::Received, date(30));
        assert_eq!((o.count, o.overdue_count), (1, 0));
    }
}
