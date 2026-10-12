//! DDPP (tax document for a received advance payment): issued automatically
//! inside the proforma payment transaction, cancelled when that payment is
//! deleted.

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    EntityTrait, QueryFilter, QuerySelect, Set,
};
use uuid::Uuid;

use super::issue::{Rate, fetch_rate, variable_symbol};
use super::{lines, view, write};
use crate::cnb::CnbClient;
use crate::document::advance::issues_ddpp;
use crate::document::ddpp::{self, Share};
use crate::document::entity::{document, document_line, payment, vat_recap};
use crate::document::line::{ItemData, LineData, MAX_INPUT, RoundingMode, Status};
use crate::error::{AppError, number_violation};
use crate::settings::doc_type::{DocType, ISSUED};
use crate::settings::repo::number_series;
use crate::space::SpaceId;

/// Whether a payment on `doc` must issue a DDPP: a native issued proforma
/// (never a received or imported one) whose supplier was a VAT payer, in
/// `standard` mode.
pub fn needed(doc: &document::Model) -> bool {
    let vat_payer = doc
        .supplier_snapshot
        .as_ref()
        .and_then(|s| s.get("vatPayer"))
        .and_then(serde_json::Value::as_bool);
    doc.doc_type == DocType::Proforma.as_str()
        && doc.direction == ISSUED
        && !doc.imported
        && doc.status == Status::Issued.as_str()
        && issues_ddpp(vat_payer, &doc.vat_mode)
}

/// The DDPP's rate, resolved before the transaction (ČNB must not be called
/// while the proforma row is locked).
pub async fn rate(
    db: &DatabaseConnection,
    cnb: &CnbClient,
    proforma: &document::Model,
    manual: Option<Decimal>,
    date: NaiveDate,
    today: NaiveDate,
) -> Result<Rate, AppError> {
    fetch_rate(db, cnb, &proforma.currency, manual, date, today).await
}

fn line_description(locale: &str, proforma_number: &str) -> String {
    match locale {
        "en" => format!("Advance payment received for proforma {proforma_number}"),
        _ => format!("Přijatá záloha k zálohové faktuře {proforma_number}"),
    }
}

/// Issue the DDPP documenting `payment` of the (locked) `proforma`.
pub async fn issue(
    txn: &DatabaseTransaction,
    proforma: &document::Model,
    payment: &payment::Model,
    rate: Rate,
) -> Result<Uuid, AppError> {
    let recap: Vec<(Decimal, Decimal, Decimal)> = vat_recap::Entity::find()
        .filter(vat_recap::Column::DocumentId.eq(proforma.id))
        .all(txn)
        .await?
        .into_iter()
        .map(|r| (r.vat_rate, r.base, r.vat))
        .collect();
    let invalid_amount = || AppError::field("amount", "invalid");
    let shares = ddpp::split(payment.amount, &recap).ok_or_else(invalid_amount)?;
    // Each share becomes a unit price (numeric(18,4), capped like any input).
    if shares.iter().any(|s| s.base.abs() >= MAX_INPUT) {
        return Err(invalid_amount());
    }
    let totals =
        ddpp::totals(&shares, rate.rate).map_err(|o| AppError::field(o.field(), "invalid"))?;
    let year = payment.date.year();
    let space = SpaceId::from_row(proforma.space_id);
    let (number, seq) =
        number_series::allocate_number(txn, space, DocType::AdvanceTaxDoc, year).await?;
    let proforma_number = proforma.number.clone().unwrap_or_default();
    let lines = item_lines(
        &shares,
        &line_description(&proforma.locale, &proforma_number),
    );
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().into();
    let mut row = document::ActiveModel {
        id: Set(id),
        space_id: Set(proforma.space_id),
        direction: Set("issued".into()),
        doc_type: Set(DocType::AdvanceTaxDoc.as_str().into()),
        status: Set(Status::Issued.as_str().into()),
        number: Set(Some(number.clone())),
        number_year: Set(Some(year)),
        number_seq: Set(Some(seq)),
        imported: Set(false),
        contact_id: Set(proforma.contact_id),
        issue_date: Set(payment.date),
        tax_point_date: Set(Some(payment.date)),
        due_date: Set(Some(payment.date)),
        currency: Set(proforma.currency.clone()),
        exchange_rate: Set(rate.rate),
        exchange_rate_date: Set(rate.date),
        exchange_rate_source: Set(rate.source.map(str::to_string)),
        locale: Set(proforma.locale.clone()),
        vat_mode: Set(proforma.vat_mode.clone()),
        bank_account_id: Set(proforma.bank_account_id),
        payment_method: Set(proforma.payment_method.clone()),
        variable_symbol: Set(Some(
            proforma
                .variable_symbol
                .clone()
                .unwrap_or_else(|| variable_symbol(&number)),
        )),
        constant_symbol: Set(proforma.constant_symbol.clone()),
        order_ref: Set(proforma.order_ref.clone()),
        header_note: Set(None),
        footer_note: Set(None),
        internal_note: Set(None),
        // The DDPP's amounts are the payment itself: never rounded.
        rounding_mode: Set(RoundingMode::None.as_str().into()),
        supplier_snapshot: Set(proforma.supplier_snapshot.clone()),
        customer_snapshot: Set(proforma.customer_snapshot.clone()),
        bank_snapshot: Set(proforma.bank_snapshot.clone()),
        paid: Set(Decimal::ZERO),
        sent_at: Set(None),
        cancelled_at: Set(None),
        cancel_reason: Set(None),
        related_document_id: Set(Some(proforma.id)),
        payment_id: Set(Some(payment.id)),
        correction_reason: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    write::apply_totals(&mut row, &totals);
    row.insert(txn)
        .await
        .map_err(|e| number_violation(e, || AppError::NumberTaken))?;
    lines::replace(txn, id, &lines).await?;
    lines::replace_recap(txn, id, &totals).await?;
    Ok(id)
}

/// One item line per rate: quantity 1, unit price = the share's base.
fn item_lines(shares: &[Share], description: &str) -> Vec<LineData> {
    shares
        .iter()
        .map(|s| {
            LineData::Item(ItemData {
                description: description.to_string(),
                quantity: Decimal::ONE,
                unit: None,
                unit_price: s.base,
                discount_pct: Decimal::ZERO,
                vat_rate: s.vat_rate,
            })
        })
        .collect()
}

/// Refuse to drop a deduction: an issued invoice deducting `id` →
/// `advance_settled`, a draft one → `advance_in_use` (remove its advance line
/// first). Cancelled invoices do not count.
pub async fn ensure_not_deducted<C: ConnectionTrait>(db: &C, id: Uuid) -> Result<(), AppError> {
    let holders: Vec<Uuid> = document_line::Entity::find()
        .select_only()
        .column(document_line::Column::DocumentId)
        .filter(document_line::Column::AdvanceDocumentId.eq(id))
        .into_tuple()
        .all(db)
        .await?;
    if holders.is_empty() {
        return Ok(());
    }
    let statuses: Vec<String> = document::Entity::find()
        .select_only()
        .column(document::Column::Status)
        .filter(document::Column::Id.is_in(holders))
        .into_tuple()
        .all(db)
        .await?;
    if statuses.iter().any(|s| s == Status::Issued.as_str()) {
        Err(AppError::AdvanceSettled)
    } else if statuses.iter().any(|s| s == Status::Draft.as_str()) {
        Err(AppError::AdvanceInUse)
    } else {
        Ok(())
    }
}

/// Cancel the DDPP of `payment_id` (if any) before the payment is deleted;
/// refused while an invoice deducts it (see [`ensure_not_deducted`]) or a
/// non-cancelled correction exists (`advance_in_use`).
pub async fn cancel_for_payment(
    txn: &DatabaseTransaction,
    payment_id: Uuid,
) -> Result<(), AppError> {
    let Some(doc) = document::Entity::find()
        .filter(document::Column::PaymentId.eq(payment_id))
        .lock_exclusive()
        .one(txn)
        .await?
    else {
        return Ok(());
    };
    if view::status(&doc)? != Status::Issued {
        return Ok(());
    }
    ensure_not_deducted(txn, doc.id).await?;
    super::ddpp_correction::ensure_uncorrected(txn, doc.id).await?;
    let reason = match doc.locale.as_str() {
        "en" => "Payment deleted",
        _ => "Platba smazána",
    };
    let now = chrono::Utc::now().into();
    let mut row: document::ActiveModel = doc.into();
    row.status = Set(Status::Cancelled.as_str().into());
    row.cancelled_at = Set(Some(now));
    row.cancel_reason = Set(Some(reason.into()));
    row.updated_at = Set(now);
    row.update(txn).await?;
    Ok(())
}
