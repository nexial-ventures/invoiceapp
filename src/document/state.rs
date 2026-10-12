//! Derived, never-stored document state: payment state and overdue.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::doc_type::DocType;
use super::line::{PaymentState, Status};

/// Payments are always positive, so `paid >= 0`. A non-positive payable with
/// nothing paid counts as settled.
pub fn payment_state(paid: Decimal, payable: Decimal) -> PaymentState {
    if paid == payable || (paid.is_zero() && payable < Decimal::ZERO) {
        PaymentState::Paid
    } else if paid.is_zero() {
        PaymentState::Unpaid
    } else if paid < payable {
        PaymentState::Partial
    } else {
        PaymentState::Overpaid
    }
}

/// A DDPP documents a payment already received: it has no payments of its own.
pub const NO_PAYMENTS_DOC_TYPE: DocType = DocType::AdvanceTaxDoc;

/// Whether payment state applies: issued, and not a DDPP.
fn payable_doc(doc_type: DocType, status: Status) -> bool {
    status == Status::Issued && doc_type != NO_PAYMENTS_DOC_TYPE
}

/// `paymentState` is only meaningful for issued documents (never a DDPP).
pub fn document_payment_state(
    doc_type: DocType,
    status: Status,
    paid: Decimal,
    payable: Decimal,
) -> Option<PaymentState> {
    payable_doc(doc_type, status).then(|| payment_state(paid, payable))
}

/// Issued, still owing (unpaid or partial ⇔ `paid < payable`) and past due.
pub fn is_overdue(
    doc_type: DocType,
    status: Status,
    paid: Decimal,
    payable: Decimal,
    due: NaiveDate,
    today: NaiveDate,
) -> bool {
    payable_doc(doc_type, status) && paid < payable && due < today
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).expect("date")
    }

    #[test]
    fn payment_states() {
        assert_eq!(payment_state(d(0), d(100)), PaymentState::Unpaid);
        assert_eq!(payment_state(d(40), d(100)), PaymentState::Partial);
        assert_eq!(payment_state(d(100), d(100)), PaymentState::Paid);
        assert_eq!(payment_state(d(101), d(100)), PaymentState::Overpaid);
        assert_eq!(payment_state(d(0), d(0)), PaymentState::Paid);
        assert_eq!(payment_state(d(0), d(-5)), PaymentState::Paid);
        assert_eq!(payment_state(d(1), d(-5)), PaymentState::Overpaid);
        assert_eq!(
            document_payment_state(DocType::Invoice, Status::Draft, d(0), d(100)),
            None
        );
        assert_eq!(
            document_payment_state(DocType::Invoice, Status::Cancelled, d(0), d(100)),
            None
        );
        assert_eq!(
            document_payment_state(DocType::AdvanceTaxDoc, Status::Issued, d(0), d(100)),
            None
        );
        assert_eq!(
            document_payment_state(DocType::Proforma, Status::Issued, d(0), d(100)),
            Some(PaymentState::Unpaid)
        );
    }

    #[test]
    fn overdue_rules() {
        let today = date(10);
        let inv = |s, paid, due| is_overdue(DocType::Invoice, s, d(paid), d(100), date(due), today);
        assert!(inv(Status::Issued, 0, 9));
        assert!(inv(Status::Issued, 50, 9));
        assert!(!inv(Status::Issued, 0, 10));
        assert!(!inv(Status::Issued, 100, 9));
        assert!(!inv(Status::Draft, 0, 9));
        assert!(!inv(Status::Cancelled, 0, 9));
        let ddpp = is_overdue(
            DocType::AdvanceTaxDoc,
            Status::Issued,
            d(0),
            d(100),
            date(1),
            today,
        );
        assert!(!ddpp);
    }
}
