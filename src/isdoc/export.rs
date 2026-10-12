//! `GET /api/documents/{id}/isdoc`: one issued document as `.isdocx`
//! (ISDOC + PDF + manifest) or plain `.isdoc`; the per-document export the
//! bulk ZIP (`export_bulk.rs`) is made of.

use std::collections::HashMap;
use std::io::{Cursor, Write as _};

use anyhow::Context as _;
use axum::extract::State;
use axum::response::Response;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::export_xml::{self, Deposit, Source, Supplement};
use crate::app::AppState;
use crate::auth::Read;
use crate::document::compute::RecapRow;
use crate::document::entity::document::{self, Column, Entity};
use crate::document::line::{LineData, PaymentMethod, Status, VatMode};
use crate::document::repo::query::{self, Full};
use crate::document::snapshot::{BankSnapshot, PartySnapshot};
use crate::error::{AppError, ErrorBody};
use crate::extract::ApiPath;
use crate::pdf::handlers::safe_filename;
use crate::pdf::{PdfService, archive};
use crate::settings::doc_type::{DocType, ISSUED, RECEIVED};
use crate::space::SpaceId;

/// One exported document file.
pub struct Exported {
    pub filename: String,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

fn snapshot<T: serde::de::DeserializeOwned>(
    v: &Option<serde_json::Value>,
) -> Result<Option<T>, AppError> {
    Ok(v.clone()
        .map(serde_json::from_value)
        .transpose()
        .context("decode document snapshot")?)
}

async fn deposits(db: &DatabaseConnection, lines: &[LineData]) -> Result<Vec<Deposit>, AppError> {
    let advances: Vec<_> = lines
        .iter()
        .filter_map(|l| match l {
            LineData::Advance(a) => Some(a),
            _ => None,
        })
        .collect();
    let ids: Vec<Uuid> = advances.iter().map(|a| a.document_id).collect();
    let docs: HashMap<Uuid, document::Model> = Entity::find()
        .filter(Column::Id.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .map(|d| (d.id, d))
        .collect();
    Ok(advances
        .into_iter()
        .map(|a| {
            let d = docs.get(&a.document_id);
            Deposit {
                number: d.and_then(|d| d.number.clone()).unwrap_or_default(),
                variable_symbol: d
                    .and_then(|d| d.variable_symbol.clone())
                    .unwrap_or_default(),
                taxed: d.is_some_and(|d| d.doc_type == DocType::AdvanceTaxDoc.as_str()),
                rows: a.recap.clone(),
            }
        })
        .collect())
}

async fn source(
    db: &DatabaseConnection,
    full: Full,
    supplement: Option<Supplement>,
) -> Result<Source, AppError> {
    let doc = &full.doc;
    let doc_type = DocType::parse_document(&doc.doc_type).context("stored document type")?;
    let supplier: PartySnapshot =
        snapshot(&doc.supplier_snapshot)?.context("issued document without supplier")?;
    let note = [
        doc.header_note.clone(),
        doc.correction_reason
            .as_ref()
            .map(|r| format!("Důvod opravy: {r}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let original = match (&full.parent, doc_type.is_correction()) {
        (Some(p), true) => p.number.clone().map(|n| (n, p.issue_date)),
        _ => None,
    };
    Ok(Source {
        id: doc.id,
        doc_type,
        number: doc.number.clone().unwrap_or_default(),
        issue_date: doc.issue_date,
        tax_point_date: doc.tax_point_date,
        due_date: doc.due_date,
        currency: doc.currency.clone(),
        rate: doc.exchange_rate,
        vat_mode: VatMode::parse(&doc.vat_mode).unwrap_or(VatMode::Standard),
        customer: snapshot(&doc.customer_snapshot)?,
        bank: snapshot::<BankSnapshot>(&doc.bank_snapshot)?,
        payment_method: PaymentMethod::parse(&doc.payment_method).unwrap_or(PaymentMethod::Other),
        variable_symbol: doc.variable_symbol.clone(),
        constant_symbol: doc.constant_symbol.clone(),
        note: (!note.is_empty()).then(|| note.join("\n")),
        deposits: deposits(db, &full.lines).await?,
        recap: full
            .recap
            .iter()
            .map(|r| RecapRow {
                vat_rate: r.vat_rate,
                base: r.base,
                vat: r.vat,
                base_czk: r.base_czk,
                vat_czk: r.vat_czk,
            })
            .collect(),
        rounding: doc.rounding,
        payable: doc.payable,
        total_czk: doc.total_czk,
        original,
        supplement,
        supplier,
        lines: full.lines,
    })
}

/// The visual form: the original of an imported document, the archive
/// (rendered on first use) of a native one. An unreachable storage fails the
/// export (503); any other failure → none.
async fn visual(
    db: &DatabaseConnection,
    pdf: &PdfService,
    doc: &document::Model,
) -> Result<Option<bytes::Bytes>, AppError> {
    let got = if doc.imported {
        match &doc.original_path {
            Some(rel) => pdf.storage().get(rel).await.map_err(AppError::from),
            None => return Ok(None),
        }
    } else {
        match archive::document_pdf(db, pdf, doc.id).await {
            Ok((_, body)) => body.into_bytes().await,
            Err(e) => Err(e),
        }
    };
    match got {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e @ AppError::StorageUnavailable(_)) => Err(e),
        Err(e) => {
            tracing::warn!(document = %doc.id, error = %e, "ISDOC export without PDF");
            Ok(None)
        }
    }
}

pub(super) type ZipOut = zip::ZipWriter<Cursor<Vec<u8>>>;

/// Stored for a payload that is compressed already (a PDF, an `.isdocx`
/// archive): deflating it again costs CPU and saves nothing.
fn compression(name: &str) -> zip::CompressionMethod {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".pdf") || lower.ends_with(".isdocx") {
        zip::CompressionMethod::Stored
    } else {
        zip::CompressionMethod::Deflated
    }
}

pub(super) fn zip_add(w: &mut ZipOut, name: &str, bytes: &[u8]) -> Result<(), AppError> {
    let opts = zip::write::SimpleFileOptions::default().compression_method(compression(name));
    w.start_file(name, opts).context("start zip entry")?;
    w.write_all(bytes).context("write zip entry")?;
    Ok(())
}

fn zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, AppError> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip_add(&mut w, name, bytes)?;
    }
    Ok(w.finish().context("finish zip")?.into_inner())
}

/// Export one issued, non-draft document (any other state → `invalid_state`).
pub async fn export(
    db: &DatabaseConnection,
    pdf: &PdfService,
    id: Uuid,
) -> Result<Exported, AppError> {
    let full = query::load(db, pdf.space_id(), id).await?;
    if full.doc.direction == RECEIVED {
        return Err(AppError::NotFound);
    }
    if full.doc.direction != ISSUED || full.doc.status == Status::Draft.as_str() {
        return Err(AppError::InvalidState);
    }
    let stem = stem(&full.doc);
    let Some(pdf_bytes) = visual(db, pdf, &full.doc).await? else {
        return plain_of(db, full).await;
    };
    let (main, pdf_name) = (format!("{stem}.isdoc"), format!("{stem}.pdf"));
    let supplement = Supplement {
        filename: pdf_name.clone(),
        sha256: Sha256::digest(&pdf_bytes).to_vec(),
    };
    let xml = export_xml::render(&source(db, full, Some(supplement)).await?);
    let manifest = export_xml::manifest(&main);
    let bytes = zip(&[
        ("manifest.xml", manifest.as_bytes()),
        (&main, xml.as_bytes()),
        (&pdf_name, &pdf_bytes),
    ])?;
    Ok(Exported {
        filename: format!("{stem}.isdocx"),
        content_type: "application/zip",
        bytes,
    })
}

/// The number, header-safe: every exported file is named after it.
pub fn stem(doc: &document::Model) -> String {
    safe_filename(doc.number.as_deref().unwrap_or("document"))
}

/// `{number}.isdoc`: the plain XML without a PDF supplement.
async fn plain_of(db: &DatabaseConnection, full: Full) -> Result<Exported, AppError> {
    let stem = stem(&full.doc);
    let xml = export_xml::render(&source(db, full, None).await?);
    Ok(Exported {
        filename: format!("{stem}.isdoc"),
        content_type: "application/xml",
        bytes: xml.into_bytes(),
    })
}

/// The plain ISDOC XML of an issued, non-draft document (the e-mail
/// attachment; the caller checks the state).
pub async fn plain(
    db: &DatabaseConnection,
    space: SpaceId,
    id: Uuid,
) -> Result<Exported, AppError> {
    plain_of(db, query::load(db, space, id).await?).await
}

pub(super) fn attachment(e: Exported) -> Response {
    crate::download::attachment(e.content_type, &e.filename, e.bytes)
}

#[utoipa::path(
    get,
    path = "/api/documents/{id}/isdoc",
    tag = "documents",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "`{number}.isdocx` (ISDOC + PDF + manifest), or `{number}.isdoc` without a PDF", content_type = "application/zip"),
        (status = 404, description = "`not_found` (also a received document)", body = ErrorBody),
        (status = 409, description = "`invalid_state` (a draft)", body = ErrorBody),
        (status = 503, description = "`storage_unavailable`", body = ErrorBody),
    )
)]
pub async fn document_isdoc(
    State(state): State<AppState>,
    access: Read,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Response, AppError> {
    let pdf = state.pdf.space(access.space())?;
    Ok(attachment(export(&state.db, &pdf, id).await?))
}
