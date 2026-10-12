//! Decimal text validators shared by request bodies (documents, catalog,
//! VAT rates, payments).

use std::str::FromStr;

use rust_decimal::Decimal;

use crate::validation::Check;

/// A decimal string with at most `dp` decimal places and `|x| < max`.
pub fn decimal(s: &str, dp: u32, max: Decimal) -> Check<Decimal> {
    let s = s.trim();
    if s.is_empty() {
        return Err("required");
    }
    let d = Decimal::from_str(s).map_err(|_| "invalid")?.normalize();
    if d.scale() > dp || d.abs() >= max {
        return Err("invalid");
    }
    Ok(d)
}

/// A percentage in 0..=100 with at most two decimal places.
pub fn parse_rate(s: &str) -> Check<Decimal> {
    let s = s.trim();
    if s.is_empty() {
        return Err("required");
    }
    let rate = Decimal::from_str(s).map_err(|_| "invalid")?.normalize();
    if rate < Decimal::ZERO || rate > Decimal::ONE_HUNDRED || rate.scale() > 2 {
        return Err("invalid");
    }
    Ok(rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::line::MAX_INPUT;

    #[test]
    fn decimal_rules() {
        assert_eq!(
            decimal(" 1.50 ", 4, MAX_INPUT),
            Ok("1.5".parse().expect("d"))
        );
        assert_eq!(decimal("1.23456", 4, MAX_INPUT), Err("invalid"));
        assert_eq!(decimal("1,5", 4, MAX_INPUT), Err("invalid"));
        assert_eq!(decimal("", 4, MAX_INPUT), Err("required"));
        assert_eq!(decimal("1000000000000", 4, MAX_INPUT), Err("invalid"));
        assert!(decimal("-999999999999.9999", 4, MAX_INPUT).is_ok());
    }

    #[test]
    fn rate_parsing() {
        assert_eq!(parse_rate("21"), Ok(Decimal::from(21)));
        assert_eq!(parse_rate(" 12.50 "), Ok(Decimal::new(125, 1)));
        assert_eq!(parse_rate("0"), Ok(Decimal::ZERO));
        assert_eq!(parse_rate("100.00"), Ok(Decimal::ONE_HUNDRED));
        assert_eq!(parse_rate(""), Err("required"));
        assert_eq!(parse_rate("abc"), Err("invalid"));
        assert_eq!(parse_rate("-1"), Err("invalid"));
        assert_eq!(parse_rate("100.01"), Err("invalid"));
        assert_eq!(parse_rate("10.125"), Err("invalid"));
    }
}
