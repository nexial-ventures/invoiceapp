pub mod bank_accounts;
pub mod categories;
pub mod company;
pub mod custom_fields;
pub mod number_series;
pub mod numbering;
pub mod vat_rates;

use uuid::Uuid;

/// Which row to promote so a non-empty group keeps exactly one default.
///
/// `rows` are `(id, is_default)` in promotion order. `None` when a default
/// already exists or the group is empty; otherwise the first row other than
/// `avoid` (the row the caller just un-flagged), falling back to `avoid`
/// itself when it is the only row.
pub fn default_to_promote(rows: &[(Uuid, bool)], avoid: Option<Uuid>) -> Option<Uuid> {
    if rows.iter().any(|(_, d)| *d) {
        return None;
    }
    rows.iter()
        .map(|(id, _)| *id)
        .find(|id| Some(*id) != avoid)
        .or_else(|| rows.first().map(|(id, _)| *id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_rules() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(default_to_promote(&[], None), None);
        assert_eq!(default_to_promote(&[(a, false), (b, true)], None), None);
        assert_eq!(default_to_promote(&[(a, false), (b, false)], None), Some(a));
        assert_eq!(
            default_to_promote(&[(a, false), (b, false)], Some(a)),
            Some(b)
        );
        assert_eq!(default_to_promote(&[(a, false)], Some(a)), Some(a));
    }
}
