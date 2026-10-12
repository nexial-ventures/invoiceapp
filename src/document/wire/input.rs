//! Document request bodies and their validation into [`DocumentData`].

use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::fields::doc_type;
pub use super::fields::{digits, exchange_rate, parse_vat_mode};
use super::imported;
use super::line_input::{LineInput, Texts, validate_lines};
use crate::currency::{LOCAL_CURRENCY, is_local};
use crate::document::advance::AdvanceCtx;
use crate::document::compute::Evaluated;
use crate::document::custom_fields::Values;
use crate::document::defaults;
use crate::document::doc_type::DocType;
use crate::document::line::Direction;
use crate::document::line::{PaymentMethod, RoundingMode, VatMode};
use crate::document::meta;
use crate::document::model::{Context, DocumentData, evaluate};
use crate::document::rounding;
use crate::error::{AppError, FieldErrors};
use crate::validation::{self as v, Check};

/// Create (`POST`, omitted fields get defaults) and update (`PUT`, replaces
/// every field) body.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DocumentInput {
    /// `invoice` | `proforma` | `simplified` on create (default `invoice`;
    /// imported: any of the seven); on update it must equal the draft's type
    /// (omitted = unchanged), corrections included.
    pub doc_type: Option<String>,
    /// Only `issued`.
    pub direction: Option<String>,
    pub contact_id: Option<Uuid>,
    pub issue_date: Option<NaiveDate>,
    pub tax_point_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub currency: Option<String>,
    /// CZK per 1 unit; manual override (null = ČNB at issue).
    #[serde(deserialize_with = "crate::num_text::opt_decimal")]
    #[schemars(with = "Option<crate::num_text::DecimalText>")]
    pub exchange_rate: Option<String>,
    pub locale: Option<String>,
    /// `standard` | `reverse_charge` | `exempt` | `non_payer`.
    pub vat_mode: Option<String>,
    pub bank_account_id: Option<Uuid>,
    /// `bank_transfer` | `cash` | `card` | `other`.
    pub payment_method: Option<String>,
    pub variable_symbol: Option<String>,
    pub constant_symbol: Option<String>,
    pub order_ref: Option<String>,
    pub header_note: Option<String>,
    pub footer_note: Option<String>,
    pub internal_note: Option<String>,
    /// `none` | `half` (nearest 0.50) | `unit` (whole units); create default:
    /// the company's default for the payment method (cash / other).
    pub rounding_mode: Option<String>,
    /// Corrections only (ignored otherwise); at most 500 characters.
    pub correction_reason: Option<String>,
    pub lines: Vec<LineInput>,
    /// Manual import (create only, default `false`; immutable).
    pub imported: Option<bool>,
    /// Imported documents only (required there, <= 40, unique per type).
    pub number: Option<String>,
    /// Imported documents only; ignored on native drafts.
    pub related_document_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    #[schema(value_type = Object)]
    pub custom_fields: Option<Values>,
}

impl DocumentInput {
    pub fn advance_ids(&self) -> Vec<Uuid> {
        advance_ids(&self.lines)
    }
}

/// The documents `advance` lines of a request reference.
pub fn advance_ids(lines: &[LineInput]) -> Vec<Uuid> {
    lines
        .iter()
        .filter(|l| l.kind == "advance")
        .filter_map(|l| l.advance_document_id)
        .collect()
}

/// `value`, else the default when allowed, else `required`.
fn or_default<T>(value: Option<T>, ctx: &Context, default: impl FnOnce() -> T) -> Check<T> {
    match value {
        Some(v) => Ok(v),
        None if ctx.apply_defaults => Ok(default()),
        None => Err("required"),
    }
}

impl DocumentInput {
    /// Normalize and validate every field (all failures at once) and run the
    /// computation rules. The bank account is checked by the repo.
    pub fn validate(self, ctx: &Context) -> Result<(DocumentData, Evaluated), AppError> {
        let mut e = FieldErrors::new();
        let existing = ctx.existing.as_ref();
        let imported = imported::flag(self.imported, existing, &mut e);
        let doc_type = match self.direction.as_deref().map(str::trim) {
            None | Some("issued") => e.check(
                "docType",
                doc_type(self.doc_type.as_deref(), existing, imported),
            ),
            Some(_) => e.check("docType", Err("invalid")),
        }
        .unwrap_or(DocType::Invoice);
        let (number, related_document_id) =
            imported::fields(imported, doc_type, &self, ctx, &mut e);
        let meta = meta::validate(
            self.category_id,
            self.custom_fields.as_ref(),
            Direction::Issued,
            &ctx.meta,
            &mut e,
        );
        if self.contact_id.is_some() && ctx.contact.is_none() {
            e.add("contactId", "invalid");
        }
        let c = ctx.contact.as_ref();
        let issue_date = e
            .check("issueDate", or_default(self.issue_date, ctx, || ctx.today))
            .unwrap_or(ctx.today);
        // A proforma is not a tax document: no tax point date.
        let tax_point_date = if doc_type == DocType::Proforma {
            if self.tax_point_date.is_some() {
                e.add("taxPointDate", "invalid");
            }
            None
        } else {
            self.tax_point_date
                .or(ctx.apply_defaults.then_some(issue_date))
        };
        let due_date = e
            .check(
                "dueDate",
                or_default(self.due_date, ctx, || {
                    defaults::due_date(c, &ctx.company, issue_date)
                }),
            )
            .unwrap_or(issue_date);
        let currency = match self.currency.as_deref().map(v::currency) {
            Some(r) => e.check("currency", r),
            None => e.check(
                "currency",
                or_default(None, ctx, || defaults::currency(c, &ctx.company)),
            ),
        }
        .unwrap_or_else(|| LOCAL_CURRENCY.into());
        let rate = e
            .check("exchangeRate", exchange_rate(self.exchange_rate.as_deref()))
            .flatten()
            .filter(|_| !is_local(&currency));
        // A native correction is bound to its original; an imported one is not.
        let correction = existing.filter(|_| doc_type.is_correction() && !imported);
        let rate = correction.map_or(rate, |x| x.exchange_rate);
        let correction_reason = if doc_type.is_correction() {
            e.check(
                "correctionReason",
                v::opt_text(self.correction_reason.as_deref(), 500),
            )
            .flatten()
        } else {
            None
        };
        let locale = match self.locale.as_deref().map(v::locale) {
            Some(r) => e.check("locale", r),
            None => e.check(
                "locale",
                or_default(None, ctx, || defaults::locale(c, &ctx.company)),
            ),
        }
        .unwrap_or_default();
        let vat_mode = match self.vat_mode.as_deref().map(parse_vat_mode) {
            Some(r) => e.check("vatMode", r),
            None => e.check(
                "vatMode",
                or_default(None, ctx, || {
                    if ctx.company.vat_payer {
                        VatMode::Standard
                    } else {
                        VatMode::NonPayer
                    }
                }),
            ),
        };
        if let (Some(x), Some(mode)) = (correction, vat_mode) {
            x.check_bound(&currency, self.contact_id, mode, &mut e);
        }
        let payment_method = match self.payment_method.as_deref() {
            Some(s) => e.check(
                "paymentMethod",
                PaymentMethod::parse(s.trim()).ok_or("invalid"),
            ),
            None if ctx.apply_defaults => Some(ctx.company.default_payment_method),
            None => e.check("paymentMethod", Err("required")),
        };
        // PUT: an omitted mode is `none` (as the 1b `roundTotal: false`), not a default.
        let rounding_mode = match (self.rounding_mode.as_deref(), payment_method) {
            (Some(s), _) => e.check("roundingMode", rounding::parse(s)),
            (None, Some(m)) if ctx.apply_defaults => Some(defaults::rounding_mode(m, &ctx.company)),
            (None, _) => Some(RoundingMode::None),
        }
        .unwrap_or(RoundingMode::None);
        let (header_default, footer_default) = match (ctx.apply_defaults, locale.as_str()) {
            (true, l) => defaults::notes(&ctx.company, l),
            (false, _) => (None, None),
        };
        let lines = validate_lines(
            self.lines,
            vat_mode,
            ctx.default_vat_rate,
            Texts::Validate,
            &mut e,
        );
        let mut data = DocumentData {
            doc_type,
            related_document_id,
            correction_reason,
            contact_id: self.contact_id,
            issue_date,
            tax_point_date,
            due_date,
            currency,
            exchange_rate: rate,
            locale,
            vat_mode: vat_mode.unwrap_or(VatMode::Standard),
            bank_account_id: self.bank_account_id,
            payment_method: payment_method.unwrap_or(PaymentMethod::BankTransfer),
            variable_symbol: e
                .check(
                    "variableSymbol",
                    digits(self.variable_symbol.as_deref(), 10),
                )
                .flatten(),
            constant_symbol: e
                .check("constantSymbol", digits(self.constant_symbol.as_deref(), 4))
                .flatten(),
            order_ref: e
                .check("orderRef", v::opt_text(self.order_ref.as_deref(), 100))
                .flatten(),
            header_note: match self.header_note.as_deref() {
                Some(s) => e.check("headerNote", v::opt_text(Some(s), 2000)).flatten(),
                None => header_default,
            },
            footer_note: match self.footer_note.as_deref() {
                Some(s) => e.check("footerNote", v::opt_text(Some(s), 2000)).flatten(),
                None => footer_default,
            },
            internal_note: e
                .check(
                    "internalNote",
                    v::opt_text(self.internal_note.as_deref(), 2000),
                )
                .flatten(),
            rounding_mode,
            lines,
            imported,
            number,
            meta,
        };
        // Line-level rules only make sense on well-formed lines.
        if !e.is_empty() {
            return Err(AppError::Validation(e));
        }
        let params = data.params();
        let adv = AdvanceCtx {
            doc_type: data.doc_type,
            vat_mode: data.vat_mode,
            document_id: existing.map(|x| x.id),
            related_document_id: data.related_document_id,
            contact_id: data.contact_id,
            currency: &data.currency,
            locale: &data.locale,
            sources: &ctx.advances,
        };
        let mut lines = std::mem::take(&mut data.lines);
        let evaluated = evaluate(&mut lines, params, &adv, ctx.exact.as_ref())?;
        data.lines = lines;
        Ok((data, evaluated))
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "input_defaults_tests.rs"]
mod defaults_tests;
