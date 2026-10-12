//! The header → where each known column and each rate column is. Pure.

use std::collections::HashMap;

use rust_decimal::Decimal;

use super::format::{AFTER_RATES, BEFORE_RATES, RatePart, base_column, rate_column};
use super::read::{Cell, FileError};
use crate::document::wire::received_input::MAX_RECAP_ROWS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateColumns {
    pub rate: Decimal,
    pub base: usize,
    pub vat: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columns {
    fixed: HashMap<&'static str, usize>,
    /// Highest rate first.
    pub rates: Vec<RateColumns>,
}

static EMPTY: Cell = Cell::Empty;

impl Columns {
    /// Unknown headers are ignored; a known header twice, a rate header
    /// that does not parse, a rate named twice or a `vat_{r}` without its
    /// `base_{r}` → `invalid_column`.
    pub fn new(header: &[String]) -> Result<Self, FileError> {
        let invalid = |name: &str| FileError::InvalidColumn(name.to_string());
        let mut fixed = HashMap::new();
        let mut bases: Vec<(Decimal, usize)> = Vec::new();
        let mut vats: Vec<(Decimal, usize, &str)> = Vec::new();
        for (i, name) in header.iter().enumerate() {
            if let Some(known) = BEFORE_RATES.iter().chain(&AFTER_RATES).find(|c| *c == name) {
                if fixed.insert(*known, i).is_some() {
                    return Err(invalid(name));
                }
                continue;
            }
            let (part, rate) = match rate_column(name) {
                None => continue,
                Some(Err(())) => return Err(invalid(name)),
                Some(Ok(x)) => x,
            };
            let taken = match part {
                RatePart::Base => bases.iter().any(|b| b.0 == rate),
                RatePart::Vat => vats.iter().any(|v| v.0 == rate),
            };
            if taken {
                return Err(invalid(name));
            }
            // A document holds as many recap rows as the received form does.
            if part == RatePart::Base && bases.len() == MAX_RECAP_ROWS {
                return Err(invalid(name));
            }
            match part {
                RatePart::Base => bases.push((rate, i)),
                RatePart::Vat => vats.push((rate, i, name)),
            }
        }
        if let Some((_, _, name)) = vats.iter().find(|v| !bases.iter().any(|b| b.0 == v.0)) {
            return Err(invalid(name));
        }
        let mut rates: Vec<RateColumns> = bases
            .into_iter()
            .map(|(rate, base)| RateColumns {
                rate,
                base,
                vat: vats.iter().find(|v| v.0 == rate).map(|v| v.1),
            })
            .collect();
        rates.sort_by_key(|r| std::cmp::Reverse(r.rate));
        Ok(Self { fixed, rates })
    }

    pub fn has(&self, name: &str) -> bool {
        self.fixed.contains_key(name)
            || self.rates.iter().any(|r| {
                base_column(r.rate) == name
                    || r.vat.is_some() && super::format::vat_column(r.rate) == name
            })
    }

    /// The cell of a fixed column (empty when the column is absent).
    pub fn cell<'a>(&self, cells: &'a [Cell], name: &str) -> &'a Cell {
        self.fixed
            .get(name)
            .and_then(|i| cells.get(*i))
            .unwrap_or(&EMPTY)
    }

    pub fn at(cells: &[Cell], i: Option<usize>) -> &Cell {
        i.and_then(|i| cells.get(i)).unwrap_or(&EMPTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[&str]) -> Result<Columns, FileError> {
        Columns::new(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn rate_columns_sorted_and_paired() {
        let c = cols(&["total", "base_0", "vat_21", "base_21", "base_12_5", "x"]).expect("cols");
        let rates: Vec<_> = c
            .rates
            .iter()
            .map(|r| (r.rate.to_string(), r.base, r.vat))
            .collect();
        assert_eq!(
            rates,
            [
                ("21".to_string(), 3, Some(2)),
                ("12.5".to_string(), 4, None),
                ("0".to_string(), 1, None)
            ]
        );
        assert!(c.has("total") && c.has("vat_21") && c.has("base_0"));
        assert!(!c.has("vat_0") && !c.has("x") && !c.has("number"));
        let cells = [Cell::Text("7".into())];
        assert_eq!(c.cell(&cells, "total"), &Cell::Text("7".into()));
        assert_eq!(c.cell(&cells, "number"), &Cell::Empty);
    }

    #[test]
    fn invalid_columns() {
        let err = |n: &str| Err(FileError::InvalidColumn(n.into()));
        assert_eq!(cols(&["base_12", "vat_21"]), err("vat_21"));
        assert_eq!(cols(&["base_abc"]), err("base_abc"));
        assert_eq!(cols(&["base_21", "base_21_0"]), err("base_21_0"));
        assert_eq!(cols(&["total", "total"]), err("total"));
        assert!(cols(&["note", "Note2", "base_21", "vat_21"]).is_ok());
        let many: Vec<String> = (0..=MAX_RECAP_ROWS).map(|r| format!("base_{r}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(cols(&names[..MAX_RECAP_ROWS]).is_ok());
        assert_eq!(cols(&names), err(&format!("base_{MAX_RECAP_ROWS}")));
    }
}
