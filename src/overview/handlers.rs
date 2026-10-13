//! `GET /api/overview`: the period figures and today's open documents in
//! one response, read in one snapshot.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use chrono::NaiveDate;
use sea_orm::{AccessMode, ConnectionTrait, IsolationLevel, TransactionTrait};
use serde::Deserialize;
use utoipa::IntoParams;

use super::aggregate::{Figures, figures};
use super::dto::Overview;
use super::open::open;
use super::period::{Chart, chart};
use super::repo;
use crate::app::AppState;
use crate::auth::Read;
use crate::document::line::Direction;
use crate::error::{AppError, ErrorBody, FieldErrors};
use crate::export::{load, period};
use crate::extract::ApiQuery;
use crate::settings::repo::{categories, company};
use crate::space::SpaceId;
use crate::time::today;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OverviewQuery {
    /// `YYYY-MM-DD`, inclusive.
    pub from: Option<String>,
    /// `YYYY-MM-DD`, inclusive; `from ≤ to ≤ from + 366 days`.
    pub to: Option<String>,
}

impl OverviewQuery {
    /// The period and its chart; `from` / `to`: `invalid`.
    pub fn check(&self) -> Result<(NaiveDate, NaiveDate, Chart), AppError> {
        let mut e = FieldErrors::new();
        let p = period::check(&mut e, self.from.as_deref(), self.to.as_deref())
            .and_then(|(from, to)| Some((from, to, chart(from, to)?)));
        if p.is_none() && e.is_empty() {
            // A valid period at the very ends of the calendar.
            e.add("from", "invalid");
            e.add("to", "invalid");
        }
        e.into_result()?;
        p.ok_or_else(|| anyhow::anyhow!("checked overview period incomplete").into())
    }
}

async fn build<C: ConnectionTrait>(
    db: &C,
    space: SpaceId,
    (from, to, chart): (NaiveDate, NaiveDate, Chart),
    today: NaiveDate,
) -> Result<Overview, AppError> {
    let vat_payer = company::get(db, space).await?.vat_payer;
    let docs = repo::docs(db, space, chart.from, chart.to).await?;
    let names: HashMap<_, _> = categories::list(db, space)
        .await?
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let open_rows = repo::open(db, space).await?;
    let Figures {
        income,
        expense,
        vat,
        months,
        categories,
    } = figures(&docs, from, to, &chart, vat_payer, &names);
    Ok(Overview {
        from,
        to,
        today,
        vat_payer,
        difference: income.amount - expense.amount,
        income,
        expense,
        vat,
        receivables: open(&open_rows, Direction::Issued, today),
        payables: open(&open_rows, Direction::Received, today),
        months,
        categories,
    })
}

#[utoipa::path(
    get,
    path = "/api/overview",
    tag = "overview",
    security(("cookie" = []), ("bearer" = [])),
    params(OverviewQuery),
    responses(
        (status = 200, description = "The period's income / expense, VAT per rate, monthly series and categories (issued-status, non-proforma documents by tax date, in CZK) and today's receivables / payables", body = Overview),
        (status = 422, description = "`from` / `to`: `invalid` (missing, not `YYYY-MM-DD`, `to < from` or more than 366 days apart)", body = ErrorBody),
    )
)]
pub async fn overview(
    State(state): State<AppState>,
    access: Read,
    ApiQuery(q): ApiQuery<OverviewQuery>,
) -> Result<Json<Overview>, AppError> {
    let checked = q.check()?;
    // One snapshot, so the period figures and the open documents agree.
    let txn = state
        .db
        .begin_with_config(
            Some(IsolationLevel::RepeatableRead),
            Some(AccessMode::ReadOnly),
        )
        .await?;
    match build(&txn, access.space(), checked, today()).await {
        Ok(o) => {
            txn.commit().await?;
            Ok(Json(o))
        }
        Err(e) => {
            load::rollback(txn, "overview").await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(from: &str, to: &str) -> OverviewQuery {
        OverviewQuery {
            from: Some(from.into()),
            to: Some(to.into()),
        }
    }

    fn invalid(r: Result<(NaiveDate, NaiveDate, Chart), AppError>) -> Vec<&'static str> {
        match r {
            Err(AppError::Validation(e)) => ["from", "to"]
                .into_iter()
                .filter(|f| e.get(f) == Some("invalid"))
                .collect(),
            other => panic!("expected validation, got {other:?}"),
        }
    }

    #[test]
    fn checks_the_period() {
        let (from, to, chart) = q("2026-01-01", "2026-03-31").check().expect("ok");
        assert_eq!(
            (from.to_string(), to.to_string()),
            ("2026-01-01".into(), "2026-03-31".into())
        );
        assert_eq!(chart.months.len(), 3);
        assert_eq!(
            invalid(q("2026-01-01", "2027-01-03").check()),
            ["from", "to"]
        );
        assert_eq!(invalid(q("x", "2026-01-31").check()), ["from"]);
        assert_eq!(invalid(OverviewQuery::default().check()), ["from", "to"]);
        assert_eq!(
            invalid(q("+262142-12-01", "+262142-12-31").check()),
            ["from", "to"]
        );
    }
}
