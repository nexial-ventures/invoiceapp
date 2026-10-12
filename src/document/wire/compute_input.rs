//! `POST /api/documents/compute` body and its validation.

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::fields::{exchange_rate, parse_vat_mode};
use super::input::advance_ids;
use super::line_input::{LineInput, Texts, validate_lines};
use crate::currency::{LOCAL_CURRENCY, is_local};
use crate::document::advance::{AdvanceCtx, AdvanceSource};
use crate::document::compute::{Evaluated, Params};
use crate::document::correction::ExactBasis;
use crate::document::doc_type::DocType;
use crate::document::line::{LineData, RoundingMode, VatMode};
use crate::document::model::{Existing, evaluate};
use crate::document::rounding;
use crate::error::{AppError, FieldErrors};
use crate::validation as v;

#[derive(Debug, Clone, Default, Deserialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ComputeInput {
    pub lines: Vec<LineInput>,
    /// Default: from the company (`standard` / `non_payer`).
    pub vat_mode: Option<String>,
    /// Default `CZK`.
    pub currency: Option<String>,
    #[serde(deserialize_with = "crate::num_text::opt_decimal")]
    #[schemars(with = "Option<crate::num_text::DecimalText>")]
    pub exchange_rate: Option<String>,
    /// `none` (default) | `half` | `unit`.
    pub rounding_mode: Option<String>,
    /// For `advance` lines: the draft being edited (its type, related
    /// proforma and own references), the contact and the description locale.
    pub document_id: Option<Uuid>,
    pub contact_id: Option<Uuid>,
    /// Default: the edited draft's type, else `invoice`.
    pub doc_type: Option<String>,
    pub locale: Option<String>,
}

impl ComputeInput {
    pub fn advance_ids(&self) -> Vec<Uuid> {
        advance_ids(&self.lines)
    }
}

/// What compute needs from the database.
pub struct ComputeCtx {
    pub vat_payer: bool,
    pub default_rate: Option<Decimal>,
    pub default_locale: String,
    /// The draft named by `documentId`, if it exists.
    pub existing: Option<Existing>,
    pub advances: HashMap<Uuid, AdvanceSource>,
    /// A native DDPP correction: the basis of its exact VAT.
    pub exact: Option<ExactBasis>,
}

impl ComputeInput {
    pub fn validate(self, ctx: &ComputeCtx) -> Result<(Vec<LineData>, Evaluated), AppError> {
        let mut e = FieldErrors::new();
        let vat_mode = match self.vat_mode.as_deref() {
            Some(s) => e.check("vatMode", parse_vat_mode(s)),
            None if ctx.vat_payer => Some(VatMode::Standard),
            None => Some(VatMode::NonPayer),
        };
        let currency = match self.currency.as_deref() {
            Some(s) => e.check("currency", v::currency(s)),
            None => Some(LOCAL_CURRENCY.into()),
        }
        .unwrap_or_default();
        let rate = e
            .check("exchangeRate", exchange_rate(self.exchange_rate.as_deref()))
            .flatten();
        // Same as save / issue: a native correction keeps its original's rate.
        let rate = match &ctx.existing {
            Some(x) if x.doc_type.is_correction() && !x.imported => x.exchange_rate,
            _ => rate,
        };
        let doc_type = match self.doc_type.as_deref() {
            Some(s) => e.check("docType", DocType::parse(s.trim()).ok_or("invalid")),
            None => Some(
                ctx.existing
                    .as_ref()
                    .map_or(DocType::Invoice, |x| x.doc_type),
            ),
        };
        let locale = match self.locale.as_deref() {
            Some(s) => e.check("locale", v::locale(s)),
            None => Some(
                ctx.existing
                    .as_ref()
                    .map_or(ctx.default_locale.clone(), |x| x.locale.clone()),
            ),
        }
        .unwrap_or_default();
        let rounding = match self.rounding_mode.as_deref() {
            Some(s) => e.check("roundingMode", rounding::parse(s)),
            None => Some(RoundingMode::None),
        };
        let mut lines = validate_lines(self.lines, vat_mode, ctx.default_rate, Texts::Skip, &mut e);
        e.into_result()?;
        let params = Params {
            vat_mode: vat_mode.unwrap_or(VatMode::Standard),
            is_czk: is_local(&currency),
            exchange_rate: rate,
            rounding: rounding.unwrap_or(RoundingMode::None),
        };
        let adv = AdvanceCtx {
            doc_type: doc_type.unwrap_or(DocType::Invoice),
            vat_mode: params.vat_mode,
            document_id: ctx.existing.as_ref().map(|x| x.id),
            related_document_id: ctx.existing.as_ref().and_then(|x| x.related_document_id),
            contact_id: self.contact_id,
            currency: &currency,
            locale: &locale,
            sources: &ctx.advances,
        };
        let evaluated = evaluate(&mut lines, params, &adv, ctx.exact.as_ref())?;
        Ok((lines, evaluated))
    }
}
