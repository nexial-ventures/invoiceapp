import { normalizeAmount, sameAmount, toCents } from "@/lib/money";
import type { FieldErrors } from "@/api/types";
import { collectErrors, nullIfEmpty, textRule } from "@/lib/formErrors";
import { defaultVatRate } from "@/features/documents/form";
import type { Document, DocumentType, ReceivedDocumentInput, VatMode } from "@/features/documents/types";
import { toMetadataDraft, toMetadataInput, validateMetadata, type MetadataDraft } from "@/features/metadata/metadata";
import type { CustomField, VatRate } from "@/features/settings/types";
import { newRecapRow, recapTotal, suggestVat, validateRecap, type RecapRowDraft } from "./recap";

/** Received-document form state; nullable wire strings are "" here. */
export interface ReceivedDraft {
  docType: DocumentType;
  contactId: string | null;
  supplierNumber: string;
  issueDate: string;
  /** Proforma: always "". */
  taxPointDate: string;
  receivedDate: string;
  /** receivedDate follows taxPointDate ?? issueDate until the user sets it. */
  receivedDateAuto: boolean;
  /** DDPP: always "". */
  dueDate: string;
  currency: string;
  /** Manual rate; "" = ČNB for receivedDate, fixed by the server. */
  exchangeRate: string;
  vatMode: VatMode;
  vatRecap: RecapRowDraft[];
  rounding: string;
  payable: string;
  /** payable follows the computed total until the user sets it (e.g. a final invoice after advances). */
  payableAuto: boolean;
  vatDeductible: boolean;
  variableSymbol: string;
  constantSymbol: string;
  supplierAccount: string;
  relatedDocumentId: string | null;
  meta: MetadataDraft;
}

export interface ReceivedContext {
  vatRates: VatRate[];
  /** Applicable custom field definitions (received). */
  fieldDefs: CustomField[];
  today: string;
}

export const hasTaxPoint = (docType: DocumentType) => docType !== "proforma";
export const hasDueDate = (docType: DocumentType) => docType !== "advance_tax_doc";
export const defaultVatDeductible = (mode: VatMode) => mode === "standard";

export function newReceivedDraft(ctx: ReceivedContext, docType: DocumentType = "invoice"): ReceivedDraft {
  const { today } = ctx;
  return {
    docType,
    contactId: null,
    supplierNumber: "",
    issueDate: today,
    taxPointDate: hasTaxPoint(docType) ? today : "",
    receivedDate: today,
    receivedDateAuto: true,
    // The supplier's due date is copied from their document, there is no sensible default.
    dueDate: "",
    currency: "CZK",
    exchangeRate: "",
    vatMode: "standard",
    vatRecap: [newRecapRow(defaultVatRate(ctx.vatRates, "standard"))],
    rounding: "0",
    payable: "",
    payableAuto: true,
    vatDeductible: true,
    variableSymbol: "",
    constantSymbol: "",
    supplierAccount: "",
    relatedDocumentId: null,
    meta: toMetadataDraft({ categoryId: null, customFields: {}, internalNote: null }, ctx.fieldDefs),
  };
}

export const defaultReceivedDate = (d: Pick<ReceivedDraft, "taxPointDate" | "issueDate">) => d.taxPointDate || d.issueDate;

/** Applies the two "follows until edited" defaults; returns `d` itself when nothing changes. */
export function applyDefaults(d: ReceivedDraft): ReceivedDraft {
  const receivedDate = d.receivedDateAuto ? defaultReceivedDate(d) : d.receivedDate;
  const total = recapTotal(d.vatRecap, d.rounding);
  const payable = d.payableAuto && total !== null ? total : d.payable;
  return receivedDate === d.receivedDate && payable === d.payable ? d : { ...d, receivedDate, payable };
}

/** Total entered on the document: Σ(base + vat) + rounding. */
export const draftTotal = (d: ReceivedDraft) => recapTotal(d.vatRecap, d.rounding);

export function toReceivedDraft(doc: Document, fieldDefs: CustomField[]): ReceivedDraft {
  const recap = doc.vatRecap ?? doc.totals.recap.map((r) => ({ rate: r.vatRate, base: r.base, vat: r.vat }));
  const vatRecap = recap.map((r) => ({ ...newRecapRow(r.rate), base: r.base, vat: r.vat, vatAuto: sameAmount(r.vat, suggestVat(r.base, r.rate)) }));
  const rounding = doc.rounding ?? doc.totals.rounding;
  const payable = doc.payable ?? doc.totals.payable;
  const receivedDate = doc.receivedDate ?? "";
  return {
    docType: doc.docType as DocumentType,
    contactId: doc.contactId,
    supplierNumber: doc.supplierNumber ?? "",
    issueDate: doc.issueDate,
    taxPointDate: doc.taxPointDate ?? "",
    receivedDate,
    receivedDateAuto: receivedDate === (doc.taxPointDate ?? doc.issueDate),
    dueDate: doc.dueDate ?? "",
    currency: doc.currency,
    // A ČNB rate was fixed by the server; only a manual one is the user's input.
    exchangeRate: doc.exchangeRateSource === "manual" ? (doc.exchangeRate ?? "") : "",
    vatMode: doc.vatMode,
    vatRecap,
    rounding,
    payable,
    payableAuto: sameAmount(payable, recapTotal(vatRecap, rounding)),
    vatDeductible: doc.vatDeductible ?? defaultVatDeductible(doc.vatMode),
    variableSymbol: doc.variableSymbol ?? "",
    constantSymbol: doc.constantSymbol ?? "",
    supplierAccount: doc.supplierAccount ?? "",
    relatedDocumentId: doc.relatedDocumentId,
    meta: toMetadataDraft(doc, fieldDefs),
  };
}

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const DECIMAL = /^\d+([.,]\d+)?$/;

export function validateReceived(d: ReceivedDraft, fieldDefs: CustomField[]): FieldErrors {
  const vs = d.variableSymbol.trim();
  const ks = d.constantSymbol.trim();
  const rate = d.exchangeRate.trim();
  const rounding = toCents(d.rounding.trim() === "" ? "0" : d.rounding);
  const payable = toCents(d.payable);
  return {
    ...collectErrors({
      contactId: !d.contactId && "required",
      supplierNumber: textRule(d.supplierNumber, { required: true, max: 40 }),
      issueDate: !DATE.test(d.issueDate) && "required",
      taxPointDate: hasTaxPoint(d.docType) && !DATE.test(d.taxPointDate) && "required",
      receivedDate: !DATE.test(d.receivedDate) && "required",
      dueDate: hasDueDate(d.docType) && (!DATE.test(d.dueDate) ? "required" : d.dueDate < d.issueDate && "invalid"),
      currency: !/^[A-Za-z]{3}$/.test(d.currency.trim()) && "invalid",
      exchangeRate: rate !== "" && (!DECIMAL.test(rate) || Number(rate.replace(",", ".")) <= 0) && "invalid",
      rounding: (rounding === null || rounding <= -10000n || rounding >= 10000n) && "invalid",
      payable: d.payable.trim() === "" ? "required" : (payable === null || payable < 0n) && "invalid",
      variableSymbol: vs !== "" && !/^\d{1,10}$/.test(vs) && "invalid",
      constantSymbol: ks !== "" && !/^\d{1,4}$/.test(ks) && "invalid",
      supplierAccount: textRule(d.supplierAccount, { max: 60 }),
    }),
    ...validateRecap(d.vatRecap, d.vatMode),
    ...validateMetadata(d.meta, fieldDefs),
  };
}

export function toReceivedInput(d: ReceivedDraft, fieldDefs: CustomField[]): ReceivedDocumentInput {
  return {
    direction: "received",
    docType: d.docType,
    contactId: d.contactId,
    supplierNumber: d.supplierNumber.trim(),
    issueDate: d.issueDate,
    taxPointDate: hasTaxPoint(d.docType) ? nullIfEmpty(d.taxPointDate) : null,
    receivedDate: d.receivedDate,
    dueDate: hasDueDate(d.docType) ? nullIfEmpty(d.dueDate) : null,
    currency: d.currency.trim().toUpperCase(),
    exchangeRate: nullIfEmpty(d.exchangeRate)?.replace(",", ".") ?? null,
    vatMode: d.vatMode,
    vatRecap: d.vatRecap.map((r) => ({
      rate: normalizeAmount(r.rate),
      base: normalizeAmount(r.base),
      vat: r.vat.trim() === "" ? "0" : normalizeAmount(r.vat),
    })),
    rounding: d.rounding.trim() === "" ? "0" : normalizeAmount(d.rounding),
    payable: normalizeAmount(d.payable),
    vatDeductible: d.vatDeductible,
    variableSymbol: nullIfEmpty(d.variableSymbol),
    constantSymbol: nullIfEmpty(d.constantSymbol),
    supplierAccount: nullIfEmpty(d.supplierAccount),
    relatedDocumentId: d.relatedDocumentId,
    ...toMetadataInput(d.meta, fieldDefs),
  };
}
