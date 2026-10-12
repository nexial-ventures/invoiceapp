//! One data row → the planned document and what the row decides beyond it
//! (category, payment, VAT deduction). Pure.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::amounts::{self, AmountError, CzkRow};
use super::cell::{
    Check, INVALID_AMOUNT, INVALID_VALUE, MISSING_FIELD, NO_VAT_ROWS, NOT_ALLOWED, Row, RowError,
    TOTAL_MISMATCH, amount_of, text_of,
};
use super::columns::Columns;
use super::format::{self as f, parse_bool};
use super::read::Cell;
use crate::currency::{LOCAL_CURRENCY, is_local};
use crate::document::doc_type::DocType;
use crate::document::line::Direction;
use crate::document::line::{PaymentMethod, VatMode};
use crate::import::model::{ContactRule, IssuedBank, Party, Plan};
use crate::validation as v;

/// The company the issued rows are ours from.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub company: Party,
    pub vat_payer: bool,
    pub locale: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mapped {
    pub plan: Plan,
    pub category: Option<String>,
    pub paid_date: Option<NaiveDate>,
    pub vat_deductible: bool,
}

fn err(code: &'static str, field: &str) -> RowError {
    RowError::new(code, field)
}

/// The counterparty as written; `None` only for an issued `simplified`
/// document without a name.
fn party(row: Row, doc_type: DocType, issued: bool, ctx: &Ctx) -> Check<Option<Party>> {
    let Some(name) = row.opt_text(f::COUNTERPARTY_NAME, 200)? else {
        return match (doc_type, issued) {
            (DocType::Simplified, true) => Ok(None),
            (DocType::Simplified, false) => Err(err(NOT_ALLOWED, f::COUNTERPARTY_NAME)),
            _ => Err(err(MISSING_FIELD, f::COUNTERPARTY_NAME)),
        };
    };
    let ico = match row.cell(f::COUNTERPARTY_ICO) {
        // A number cell loses the IČO's leading zeros.
        Cell::Number(n) => Some(format!("{:0>8}", n.normalize())),
        c => text_of(c),
    };
    let ico = v::opt_ico(ico.as_deref()).map_err(|_| err(INVALID_VALUE, f::COUNTERPARTY_ICO))?;
    if ico.is_some() && ico == ctx.company.ico {
        return Err(err(INVALID_VALUE, f::COUNTERPARTY_ICO));
    }
    let dic = v::opt_dic(row.text(f::COUNTERPARTY_DIC).as_deref())
        .map_err(|_| err(INVALID_VALUE, f::COUNTERPARTY_DIC))?;
    let country = v::country(&row.text(f::COUNTERPARTY_COUNTRY).unwrap_or_default())
        .map_err(|_| err(INVALID_VALUE, f::COUNTERPARTY_COUNTRY))?;
    Ok(Some(Party {
        name,
        ico,
        dic,
        street: row
            .opt_text(f::COUNTERPARTY_STREET, 200)?
            .unwrap_or_default(),
        city: row.opt_text(f::COUNTERPARTY_CITY, 100)?.unwrap_or_default(),
        zip: row.opt_text(f::COUNTERPARTY_ZIP, 20)?.unwrap_or_default(),
        country,
        ..Default::default()
    }))
}

/// The document currency and its rate (`None` for CZK).
fn currency(row: Row) -> Check<(String, Option<Decimal>)> {
    let currency = match row.text(f::CURRENCY) {
        None => LOCAL_CURRENCY.to_string(),
        Some(c) => v::currency(&c).map_err(|_| err(INVALID_VALUE, f::CURRENCY))?,
    };
    let rate = amount_of(row.cell(f::EXCHANGE_RATE), f::EXCHANGE_RATE, 6)?;
    if is_local(&currency) {
        return match rate {
            Some(r) if r != Decimal::ONE => Err(err(INVALID_VALUE, f::EXCHANGE_RATE)),
            _ => Ok((currency, None)),
        };
    }
    match rate {
        None => Err(err(MISSING_FIELD, f::EXCHANGE_RATE)),
        Some(r) if r <= Decimal::ZERO => Err(err(INVALID_AMOUNT, f::EXCHANGE_RATE)),
        Some(r) => Ok((currency, Some(r))),
    }
}

/// Recap rows as written (a rate takes part when its base or VAT is set).
fn recap(row: Row) -> Check<Vec<(CzkRow, String, String)>> {
    let mut out = Vec::new();
    for rc in &row.cols.rates {
        let (bname, vname) = (f::base_column(rc.rate), f::vat_column(rc.rate));
        let base = amount_of(Columns::at(row.cells, Some(rc.base)), &bname, 2)?;
        let vat = amount_of(Columns::at(row.cells, rc.vat), &vname, 2)?;
        if base.is_some() || vat.is_some() {
            let r = CzkRow {
                rate: rc.rate,
                base: base.unwrap_or_default(),
                vat: vat.unwrap_or_default(),
            };
            out.push((r, bname, vname));
        }
    }
    Ok(out)
}

pub fn map(row: Row, ctx: &Ctx) -> Check<Mapped> {
    let direction = row
        .choice(f::DIRECTION, Direction::parse)?
        .ok_or_else(|| err(MISSING_FIELD, f::DIRECTION))?;
    let doc_type = row
        .choice(f::DOC_TYPE, DocType::parse_document)?
        .ok_or_else(|| err(MISSING_FIELD, f::DOC_TYPE))?;
    let issued = direction == Direction::Issued;
    let number = row.req_text(
        if issued {
            f::NUMBER
        } else {
            f::SUPPLIER_NUMBER
        },
        40,
    )?;
    let related = row.opt_text(f::RELATED_NUMBER, 40)?;
    let issue_date = row
        .date(f::ISSUE_DATE)?
        .ok_or_else(|| err(MISSING_FIELD, f::ISSUE_DATE))?;
    let tax_point_date = match (doc_type, row.date(f::TAX_DATE)?) {
        (DocType::Proforma, Some(_)) => return Err(err(NOT_ALLOWED, f::TAX_DATE)),
        (DocType::Proforma, None) => None,
        (_, t) => Some(t.unwrap_or(issue_date)),
    };
    let due_date = match (issued, doc_type, row.date(f::DUE_DATE)?) {
        (false, DocType::AdvanceTaxDoc, d) => d,
        (_, _, d) => Some(d.unwrap_or(issue_date)),
    };
    let received_date = if issued {
        None
    } else {
        Some(
            row.date(f::RECEIVED_DATE)?
                .or(tax_point_date)
                .unwrap_or(issue_date),
        )
    };
    let counterparty = party(row, doc_type, issued, ctx)?;
    let (currency, rate) = currency(row)?;
    // As a native issued document: a non-payer company issues `non_payer`.
    let default_mode = if issued && !ctx.vat_payer {
        VatMode::NonPayer
    } else {
        VatMode::Standard
    };
    let vat_mode = row
        .choice(f::VAT_MODE, VatMode::parse)?
        .unwrap_or(default_mode);
    let mut rows = recap(row)?;
    let mut rounding = row.amount(f::ROUNDING)?.unwrap_or_default();
    let mut total = row
        .amount(f::TOTAL)?
        .ok_or_else(|| err(MISSING_FIELD, f::TOTAL))?;
    // Credit notes and DDPP corrections are written negative.
    if doc_type.sign() < 0 && total < Decimal::ZERO {
        total = -total;
        rounding = -rounding;
        for (r, _, _) in &mut rows {
            r.base = -r.base;
            r.vat = -r.vat;
        }
    }
    for (r, bname, vname) in &rows {
        if r.base < Decimal::ZERO {
            return Err(err(INVALID_AMOUNT, bname));
        }
        if r.vat < Decimal::ZERO {
            return Err(err(INVALID_AMOUNT, vname));
        }
    }
    if total < Decimal::ZERO {
        return Err(err(INVALID_AMOUNT, f::TOTAL));
    }
    if rounding.abs() >= Decimal::ONE_HUNDRED {
        return Err(err(INVALID_AMOUNT, f::ROUNDING));
    }
    if rows.is_empty() {
        return Err(RowError {
            code: NO_VAT_ROWS,
            field: None,
        });
    }
    if let Some((_, _, vname)) = rows
        .iter()
        .find(|(r, _, _)| !vat_mode.charges_vat() && !r.vat.is_zero())
    {
        return Err(err(NOT_ALLOWED, vname));
    }
    let czk: Vec<CzkRow> = rows.iter().map(|(r, _, _)| *r).collect();
    let totals = amounts::totals(&czk, rounding, total, rate).map_err(|e| match e {
        AmountError::Mismatch => err(TOTAL_MISMATCH, f::TOTAL),
        AmountError::Overflow => err(INVALID_AMOUNT, f::EXCHANGE_RATE),
    })?;
    let paid_date = row.date(f::PAID_DATE)?;
    if paid_date.is_some() && doc_type == DocType::AdvanceTaxDoc {
        return Err(err(NOT_ALLOWED, f::PAID_DATE));
    }
    let variable_symbol = row
        .opt_text(f::VARIABLE_SYMBOL, 10)?
        .filter(|s| s.bytes().all(|b| b.is_ascii_digit()));
    if variable_symbol.is_none() && row.text(f::VARIABLE_SYMBOL).is_some() {
        return Err(err(INVALID_VALUE, f::VARIABLE_SYMBOL));
    }
    let vat_deductible = !issued
        && row
            .choice(f::VAT_DEDUCTIBLE, parse_bool)?
            .unwrap_or(vat_mode == VatMode::Standard);
    let category = row.opt_text(f::CATEGORY, 100)?;
    let note = row.opt_text(f::NOTE, 2000)?;
    let (supplier, customer, vat_payer) = if issued {
        (ctx.company.clone(), counterparty, Some(ctx.vat_payer))
    } else {
        let supplier = counterparty.ok_or_else(|| err(MISSING_FIELD, f::COUNTERPARTY_NAME))?;
        (supplier, None, None)
    };
    let gross = totals.payable;
    Ok(Mapped {
        plan: Plan {
            direction,
            doc_type,
            number,
            supplier,
            customer,
            vat_payer,
            issue_date,
            tax_point_date,
            due_date,
            received_date,
            currency,
            rate,
            vat_mode,
            lines: Vec::new(),
            totals,
            gross,
            payment_method: PaymentMethod::BankTransfer,
            variable_symbol,
            constant_symbol: None,
            bank: None,
            note,
            locale: ctx.locale.clone(),
            original_ref: related,
            warnings: Vec::new(),
            contact_rule: ContactRule::IcoDicName,
            issued_bank: IssuedBank::CurrencyDefault,
        },
        category,
        paid_date,
        vat_deductible,
    })
}

#[cfg(test)]
#[path = "row_tests.rs"]
mod tests;
