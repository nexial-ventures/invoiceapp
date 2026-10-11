//! Rounding of the payable total (`docs/api/defaults.md`): none, to the
//! nearest 0.50 or to whole units of the document currency, half away from
//! zero — symmetric, so a credit note rounds like its original.

use rust_decimal::{Decimal, RoundingStrategy};

use super::compute::round2;
pub use super::line::RoundingMode;
use crate::validation::Check;

impl RoundingMode {
    /// `total` rounded by the mode (2 dp); `None` on overflow.
    pub fn apply(self, total: Decimal) -> Option<Decimal> {
        let whole =
            |x: Decimal| x.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
        let r = match self {
            RoundingMode::None => total,
            RoundingMode::Unit => whole(total),
            RoundingMode::Half => whole(total.checked_mul(Decimal::TWO)?) / Decimal::TWO,
        };
        Some(round2(r))
    }

    /// The mode of a document whose rounding *amount* was given (received,
    /// imported): `none` without rounding, else the coarsest mode whose step
    /// `total + rounding` is a multiple of; an "odd" amount keeps `none`.
    pub fn derive(total: Decimal, rounding: Decimal) -> RoundingMode {
        if rounding.is_zero() {
            return RoundingMode::None;
        }
        let Some(payable) = total.checked_add(rounding) else {
            return RoundingMode::None;
        };
        if payable.fract().is_zero() {
            RoundingMode::Unit
        } else if (payable * Decimal::TWO).fract().is_zero() {
            RoundingMode::Half
        } else {
            RoundingMode::None
        }
    }
}

/// A `roundingMode`-like request field (trimmed).
pub fn parse(s: &str) -> Check<RoundingMode> {
    RoundingMode::parse(s.trim()).ok_or("invalid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    fn check(mode: RoundingMode, cases: &[(&str, &str)]) {
        for (total, payable) in cases {
            let got = mode.apply(d(total)).expect("fits");
            assert_eq!(got, d(payable), "{mode:?} {total}");
            assert_eq!(got.scale(), 2, "{mode:?} {total} keeps 2 dp");
        }
    }

    #[test]
    fn none_keeps_the_total() {
        check(
            RoundingMode::None,
            &[("1234.56", "1234.56"), ("-0.01", "-0.01"), ("0", "0.00")],
        );
    }

    #[test]
    fn unit_rounds_half_away_from_zero() {
        check(
            RoundingMode::Unit,
            &[
                ("1234.49", "1234.00"),
                ("1234.50", "1235.00"),
                ("1234.51", "1235.00"),
                ("-1234.49", "-1234.00"),
                ("-1234.50", "-1235.00"),
                ("-0.50", "-1.00"),
                ("0.49", "0.00"),
                ("7", "7.00"),
            ],
        );
    }

    #[test]
    fn half_rounds_to_the_nearest_half_unit() {
        check(
            RoundingMode::Half,
            &[
                ("100.24", "100.00"),
                ("100.25", "100.50"),
                ("100.26", "100.50"),
                ("100.74", "100.50"),
                ("100.75", "101.00"),
                ("100.50", "100.50"),
                ("-100.25", "-100.50"),
                ("-100.24", "-100.00"),
                ("-100.75", "-101.00"),
                ("0.24", "0.00"),
            ],
        );
    }

    #[test]
    fn rounding_is_symmetric() {
        for mode in RoundingMode::ALL {
            for x in ["12.25", "12.75", "0.50", "99.99", "1.01"] {
                let pos = mode.apply(d(x)).expect("fits");
                let neg = mode.apply(-d(x)).expect("fits");
                assert_eq!(neg, -pos, "{mode:?} {x}");
            }
        }
    }

    #[test]
    fn derives_the_mode_of_given_amounts() {
        let cases = [
            ("1234.56", "0", RoundingMode::None),
            ("1234.56", "0.44", RoundingMode::Unit),
            ("1234.56", "-0.06", RoundingMode::Half),
            ("1234.40", "0.10", RoundingMode::Half),
            ("1234.56", "0.01", RoundingMode::None),
            ("-1234.56", "-0.44", RoundingMode::Unit),
            ("-1234.56", "0.06", RoundingMode::Half),
            ("100.00", "0", RoundingMode::None),
        ];
        for (total, rounding, mode) in cases {
            assert_eq!(
                RoundingMode::derive(d(total), d(rounding)),
                mode,
                "{total} {rounding}"
            );
        }
    }

    #[test]
    fn parses_the_wire_values() {
        assert_eq!(parse(" half "), Ok(RoundingMode::Half));
        assert_eq!(parse("unit"), Ok(RoundingMode::Unit));
        assert_eq!(parse("none"), Ok(RoundingMode::None));
        assert_eq!(parse("whole"), Err("invalid"));
    }
}
