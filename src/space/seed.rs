//! Everything a fresh space starts with — the rows a single-tenant instance
//! used to get from its migrations: the company profile row, the default VAT
//! rates, all fourteen number series and the accounting settings row — plus
//! the hidden limits row with the defaults.

use sea_orm::{ConnectionTrait, DatabaseTransaction, Statement};
use uuid::Uuid;

use crate::error::AppError;
use crate::settings::doc_type::DocType;
use crate::space::SpaceId;

/// The default pattern of each number series.
pub fn default_pattern(doc_type: DocType) -> &'static str {
    match doc_type {
        DocType::Invoice => "{YYYY}{NNNN}",
        DocType::CreditNote => "D{YYYY}{NNNN}",
        DocType::DebitNote => "V{YYYY}{NNNN}",
        DocType::Proforma => "Z{YYYY}{NNNN}",
        DocType::AdvanceTaxDoc => "DP{YYYY}{NNNN}",
        DocType::AdvanceCreditNote => "OP{YYYY}{NNNN}",
        DocType::Simplified => "ZD{YYYY}{NNNN}",
        DocType::Received => "P{YYYY}{NNNN}",
        DocType::ReceivedCreditNote => "PD{YYYY}{NNNN}",
        DocType::ReceivedDebitNote => "PV{YYYY}{NNNN}",
        DocType::ReceivedProforma => "PZ{YYYY}{NNNN}",
        DocType::ReceivedAdvanceTaxDoc => "PDP{YYYY}{NNNN}",
        DocType::ReceivedAdvanceCreditNote => "POP{YYYY}{NNNN}",
        DocType::ReceivedSimplified => "PZD{YYYY}{NNNN}",
    }
}

/// (rate, label, default, position) of the seeded VAT rates.
const VAT_RATES: [(i32, &str, bool, i32); 3] = [
    (21, "Základní", true, 1),
    (12, "Snížená", false, 2),
    (0, "Nulová", false, 3),
];

/// Insert the seeds of `space` inside the transaction that creates it.
pub async fn seed(txn: &DatabaseTransaction, space: SpaceId) -> Result<(), AppError> {
    let exec = |sql: &str, values: Vec<sea_orm::Value>| {
        Statement::from_sql_and_values(txn.get_database_backend(), sql, values)
    };
    txn.execute(exec(
        "INSERT INTO company (space_id) VALUES ($1)",
        vec![space.into()],
    ))
    .await?;
    txn.execute(exec(
        "INSERT INTO accounting_settings (space_id) VALUES ($1)",
        vec![space.into()],
    ))
    .await?;
    super::limits::seed(txn, space).await?;
    for (rate, label, is_default, position) in VAT_RATES {
        txn.execute(exec(
            "INSERT INTO vat_rates (id, space_id, rate, label, is_default, position) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            vec![
                Uuid::new_v4().into(),
                space.into(),
                rate.into(),
                label.into(),
                is_default.into(),
                position.into(),
            ],
        ))
        .await?;
    }
    for doc_type in DocType::ALL {
        txn.execute(exec(
            "INSERT INTO number_series (id, space_id, doc_type, pattern) VALUES ($1, $2, $3, $4)",
            vec![
                Uuid::new_v4().into(),
                space.into(),
                doc_type.as_str().into(),
                default_pattern(doc_type).into(),
            ],
        ))
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::pattern::Pattern;

    #[test]
    fn every_default_pattern_parses_and_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for d in DocType::ALL {
            let p = default_pattern(d);
            assert!(Pattern::parse(p).is_ok(), "{p}");
            assert!(seen.insert(p), "duplicate {p}");
        }
    }
}
