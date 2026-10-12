//! The local (accounting) currency: what VAT is reported and the books are
//! kept in, the currency ČNB rates convert to and the `…Czk` amounts hold.

pub const LOCAL_CURRENCY: &str = "CZK";

/// Whether `currency` (an ISO code, already normalized) is the local one.
pub fn is_local(currency: &str) -> bool {
    currency == LOCAL_CURRENCY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_normalized_local_code_is_local() {
        assert!(is_local("CZK"));
        assert!(!is_local("EUR"));
        assert!(!is_local("czk"));
    }
}
