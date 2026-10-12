//! The Money S3 VAT summary of a document (`souhrnDPHType` + `Celkem`): the
//! recap in Money's slots (`Zaklad0`, `Zaklad5`/`DPH5` = 12 %,
//! `Zaklad22`/`DPH22` = 21 %, `SeznamDalsiSazby` for any other rate). Pure.

use rust_decimal::Decimal;

use super::doc::{Rates, amount};
use crate::isdoc::xml::Xml;

/// Most `DalsiSazba` next to the standard slots: "seznam dalších sazeb může
/// standardně obsahovat max. 4 sazby" (`__Comtypes.xsd`).
pub const MAX_OTHER: usize = 4;

/// `HladinaDPH` of a further rate: 0 nulová, 2 základní (≥ 20 %), else 1
/// snížená.
fn level(rate: Decimal) -> &'static str {
    if rate.is_zero() {
        "0"
    } else if rate >= Decimal::from(20) {
        "2"
    } else {
        "1"
    }
}

/// `SouhrnDPH` (every amount × `sign`) and then `Celkem`, the sum of what
/// was written. The summary has no rounding element and Money ignores
/// `Celkem` on import (it sums the summary), so the document's `rounding`
/// goes into `Zaklad0` — that way Money's total is our payable.
pub fn write(x: &mut Xml, rates: &Rates, rounding: Decimal, sign: Decimal) -> anyhow::Result<()> {
    anyhow::ensure!(
        rates.other.len() <= MAX_OTHER,
        "{} VAT rates other than 0 / 12 / 21 %, Money S3 takes at most {MAX_OTHER}",
        rates.other.len()
    );
    let mut total = Decimal::ZERO;
    let mut put = |x: &mut Xml, name: &str, v: Decimal| {
        let v = (v * sign).round_dp(2);
        total += v;
        x.leaf(name, amount(v));
    };
    x.open("SouhrnDPH", &[]);
    // Without any of Zaklad0 / Zaklad5 / Zaklad22 Money would read the zero
    // and standard rates from SeznamDalsiSazby (`__Comtypes.xsd`), so a
    // document with further rates always has Zaklad0.
    if rates.none.is_some() || !rounding.is_zero() || !rates.other.is_empty() {
        put(x, "Zaklad0", rates.none.unwrap_or_default() + rounding);
    }
    if let Some((base, _)) = rates.low {
        put(x, "Zaklad5", base);
    }
    if let Some((base, _)) = rates.high {
        put(x, "Zaklad22", base);
    }
    if let Some((_, vat)) = rates.low {
        put(x, "DPH5", vat);
    }
    if let Some((_, vat)) = rates.high {
        put(x, "DPH22", vat);
    }
    if !rates.other.is_empty() {
        x.open("SeznamDalsiSazby", &[]);
        for &(rate, base, vat) in &rates.other {
            x.open("DalsiSazba", &[])
                .leaf("HladinaDPH", level(rate))
                .leaf("Sazba", rate.to_string());
            put(x, "Zaklad", base);
            put(x, "DPH", vat);
            x.close();
        }
        x.close();
    }
    x.close();
    x.leaf("Celkem", amount(total));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::test_doc::d;

    fn render(rates: &Rates, rounding: &str, sign: &str) -> anyhow::Result<String> {
        let mut x = Xml::fragment();
        write(&mut x, rates, d(rounding), d(sign))?;
        Ok(x.unclosed())
    }

    #[test]
    fn slots_rounding_and_total() {
        let r = Rates {
            none: Some(d("50")),
            low: Some((d("100"), d("12"))),
            high: Some((d("1000"), d("210"))),
            other: vec![(d("10"), d("100"), d("10"))],
        };
        assert_eq!(
            render(&r, "-0.40", "1").expect("ok"),
            "<SouhrnDPH><Zaklad0>49.60</Zaklad0><Zaklad5>100.00</Zaklad5>\
             <Zaklad22>1000.00</Zaklad22><DPH5>12.00</DPH5><DPH22>210.00</DPH22>\
             <SeznamDalsiSazby><DalsiSazba><HladinaDPH>1</HladinaDPH><Sazba>10</Sazba>\
             <Zaklad>100.00</Zaklad><DPH>10.00</DPH></DalsiSazba></SeznamDalsiSazby></SouhrnDPH>\
             <Celkem>1481.60</Celkem>"
        );
    }

    #[test]
    fn only_further_rates_keep_zaklad0() {
        let r = Rates {
            other: vec![(d("10"), d("100"), d("10")), (d("23"), d("10"), d("2.30"))],
            ..Rates::default()
        };
        assert_eq!(
            render(&r, "0", "1").expect("ok"),
            "<SouhrnDPH><Zaklad0>0.00</Zaklad0><SeznamDalsiSazby><DalsiSazba>\
             <HladinaDPH>1</HladinaDPH><Sazba>10</Sazba><Zaklad>100.00</Zaklad><DPH>10.00</DPH>\
             </DalsiSazba><DalsiSazba><HladinaDPH>2</HladinaDPH><Sazba>23</Sazba>\
             <Zaklad>10.00</Zaklad><DPH>2.30</DPH></DalsiSazba></SeznamDalsiSazby></SouhrnDPH>\
             <Celkem>122.30</Celkem>"
        );
        assert_eq!(level(d("0")), "0");
        assert_eq!(level(d("20")), "2");
        assert_eq!(level(d("15")), "1");
    }

    #[test]
    fn negative_and_rounding_only() {
        let r = Rates {
            high: Some((d("100"), d("21"))),
            ..Rates::default()
        };
        assert_eq!(
            render(&r, "0", "-1").expect("ok"),
            "<SouhrnDPH><Zaklad22>-100.00</Zaklad22><DPH22>-21.00</DPH22></SouhrnDPH>\
             <Celkem>-121.00</Celkem>"
        );
        // A credit note of 12.40 rounded to 12: the rounding (stored like
        // the amounts, positive) is negated with them.
        let r = Rates {
            low: Some((d("11.07"), d("1.33"))),
            ..Rates::default()
        };
        assert_eq!(
            render(&r, "-0.40", "-1").expect("ok"),
            "<SouhrnDPH><Zaklad0>0.40</Zaklad0><Zaklad5>-11.07</Zaklad5><DPH5>-1.33</DPH5>\
             </SouhrnDPH><Celkem>-12.00</Celkem>"
        );
        assert_eq!(
            render(&Rates::default(), "0", "1").expect("ok"),
            "<SouhrnDPH></SouhrnDPH><Celkem>0.00</Celkem>"
        );
    }

    #[test]
    fn too_many_other_rates() {
        let other = |n: usize| Rates {
            other: (1..=n)
                .map(|i| (Decimal::from(i), d("1"), d("0.01")))
                .collect(),
            ..Rates::default()
        };
        assert!(render(&other(4), "0", "1").is_ok());
        let e = render(&other(5), "0", "1").expect_err("five");
        assert_eq!(
            e.to_string(),
            "5 VAT rates other than 0 / 12 / 21 %, Money S3 takes at most 4"
        );
    }
}
