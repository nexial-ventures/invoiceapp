import type { DocLocale, FieldErrors } from "@/api/types";
import { collectErrors, nullIfEmpty, textRule } from "@/lib/formErrors";
import type { Contact } from "@/features/contacts/types";
import { toMetadataDraft, toMetadataInput, validateMetadata, type MetadataDraft } from "@/features/metadata/metadata";
import type { BankAccount, Company, CustomField, VatRate } from "@/features/settings/types";
import { changeLocale, defaultNote, defaultRounding } from "./defaults";
import { isCorrection } from "./docTypes";
import { toLineDraft, toWireLine, type LineDraft } from "./lines";
import type { ComputeRequest, Document, DocumentInput, EditableDocType, PaymentMethod, RoundingMode, VatMode } from "./types";

/** Editor form state; nullable wire strings are "" here. */
export interface DocumentDraft {
  /** Fixed for the life of a draft: a proforma has no tax point, a correction keeps its original's rate. */
  docType: EditableDocType;
  contactId: string | null;
  issueDate: string;
  taxPointDate: string;
  dueDate: string;
  currency: string;
  exchangeRate: string;
  locale: DocLocale;
  vatMode: VatMode;
  bankAccountId: string;
  paymentMethod: PaymentMethod;
  variableSymbol: string;
  constantSymbol: string;
  orderRef: string;
  headerNote: string;
  footerNote: string;
  roundingMode: RoundingMode;
  /** New draft: the rounding follows the payment method's company default until the user picks one. */
  roundingAuto: boolean;
  correctionReason: string;
  /**
   * Native: read-only, the proforma a final invoice settles (its non-payer advance line references it).
   * Imported: picked by the user (DDPP / final invoice → proforma, credit / debit note → invoice or simplified,
   * DDPP correction → DDPP).
   */
  relatedDocumentId: string | null;
  /** Manual import of an existing document; fixed for the life of the draft. */
  imported: boolean;
  /** Imported only: the document's own number. */
  number: string;
  /** Category, custom fields, internal note. */
  meta: MetadataDraft;
  lines: LineDraft[];
}

/** Settings the defaults depend on. */
export interface DraftContext {
  company: Company;
  vatRates: VatRate[];
  bankAccounts: BankAccount[];
  /** Custom field definitions applicable to issued documents. */
  fieldDefs: CustomField[];
  today: string;
}

/** Local calendar date as `YYYY-MM-DD`. */
export function todayIso(now = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

export function addDays(date: string, days: number): string {
  const d = new Date(`${date}T00:00:00Z`);
  if (Number.isNaN(d.getTime())) return date;
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

export function defaultBankAccountId(accounts: BankAccount[], currency: string): string {
  const matching = accounts.filter((a) => a.currency === currency);
  return (matching.find((a) => a.isDefault) ?? matching[0])?.id ?? "";
}

/** Rate for a new item line: the default active rate, "0" for non-payers. */
export function defaultVatRate(rates: VatRate[], vatMode: VatMode): string {
  if (vatMode === "non_payer") return "0";
  const active = rates.filter((r) => r.active);
  return (active.find((r) => r.isDefault) ?? active[0])?.rate ?? "0";
}

/**
 * Mirrors the server's create defaults so the form shows them before the first save.
 * Natively only invoices, proformas and simplified documents start empty; an import may be any type. */
export function newDocumentDraft(ctx: DraftContext, docType: EditableDocType = "invoice", imported = false): DocumentDraft {
  const { company, bankAccounts, today } = ctx;
  const locale = company.defaultLocale;
  const paymentMethod = company.defaultPaymentMethod;
  return {
    docType,
    contactId: null,
    issueDate: today,
    // A proforma is not a tax document: the server rejects a tax point date.
    taxPointDate: docType === "proforma" ? "" : today,
    dueDate: addDays(today, company.defaultDueDays),
    currency: company.defaultCurrency,
    exchangeRate: "",
    locale,
    vatMode: company.vatPayer ? "standard" : "non_payer",
    bankAccountId: defaultBankAccountId(bankAccounts, company.defaultCurrency),
    paymentMethod,
    variableSymbol: "",
    constantSymbol: "",
    orderRef: "",
    headerNote: defaultNote(company, "header", locale),
    footerNote: defaultNote(company, "footer", locale),
    roundingMode: defaultRounding(company, paymentMethod),
    roundingAuto: true,
    correctionReason: "",
    relatedDocumentId: null,
    imported,
    number: "",
    meta: toMetadataDraft({ categoryId: null, customFields: {}, internalNote: null }, ctx.fieldDefs),
    lines: [],
  };
}

/**
 * Picking a customer applies its due days, locale (and that locale's default notes) and
 * currency (and that currency's bank account).
 */
export function applyContact(draft: DocumentDraft, contact: Contact, ctx: DraftContext): DocumentDraft {
  const currency = contact.defaultCurrency ?? ctx.company.defaultCurrency;
  const next: DocumentDraft = {
    ...changeLocale(draft, contact.defaultLocale ?? ctx.company.defaultLocale, ctx.company),
    contactId: contact.id,
    dueDate: addDays(draft.issueDate, contact.defaultDueDays ?? ctx.company.defaultDueDays),
  };
  return currency === draft.currency ? next : changeCurrency(next, currency, ctx.bankAccounts);
}

/**
 * Keeps currency-dependent fields consistent: bank account and the manual rate —
 * a rate for one currency is meaningless for another.
 */
export function changeCurrency(draft: DocumentDraft, currency: string, accounts: BankAccount[]): DocumentDraft {
  const code = currency.trim().toUpperCase();
  const account = accounts.find((a) => a.id === draft.bankAccountId);
  return {
    ...draft,
    currency: code,
    bankAccountId: account?.currency === code ? draft.bankAccountId : defaultBankAccountId(accounts, code),
    exchangeRate: code === draft.currency ? draft.exchangeRate : "",
  };
}

export function toDraft(doc: Document, fieldDefs: CustomField[] = []): DocumentDraft {
  return {
    // Only drafts reach the editor: native DDPPs never are, imported ones may be.
    docType: doc.docType as EditableDocType,
    contactId: doc.contactId,
    issueDate: doc.issueDate,
    taxPointDate: doc.taxPointDate ?? "",
    dueDate: doc.dueDate ?? "",
    currency: doc.currency,
    exchangeRate: doc.exchangeRate ?? "",
    locale: doc.locale,
    vatMode: doc.vatMode,
    bankAccountId: doc.bankAccountId ?? "",
    paymentMethod: doc.paymentMethod,
    variableSymbol: doc.variableSymbol ?? "",
    constantSymbol: doc.constantSymbol ?? "",
    orderRef: doc.orderRef ?? "",
    headerNote: doc.headerNote ?? "",
    footerNote: doc.footerNote ?? "",
    roundingMode: doc.roundingMode,
    // PUT applies no defaults: a saved draft keeps what it has.
    roundingAuto: false,
    correctionReason: doc.correctionReason ?? "",
    relatedDocumentId: doc.relatedDocumentId,
    imported: doc.imported,
    number: doc.imported ? (doc.number ?? "") : "",
    meta: toMetadataDraft(doc, fieldDefs),
    lines: doc.lines.map(toLineDraft),
  };
}

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const DECIMAL = /^-?\d+([.,]\d+)?$/;

export function validateDocument(d: DocumentDraft, fieldDefs: CustomField[] = []): FieldErrors {
  const vs = d.variableSymbol.trim();
  const ks = d.constantSymbol.trim();
  const rate = d.exchangeRate.trim();
  const errors = collectErrors({
    issueDate: !DATE.test(d.issueDate) && "required",
    dueDate: !DATE.test(d.dueDate) ? "required" : d.dueDate < d.issueDate && "invalid",
    taxPointDate: d.taxPointDate !== "" && !DATE.test(d.taxPointDate) && "invalid",
    currency: !/^[A-Za-z]{3}$/.test(d.currency.trim()) && "invalid",
    exchangeRate: rate !== "" && !DECIMAL.test(rate) && "invalid",
    variableSymbol: vs !== "" && !/^\d{1,10}$/.test(vs) && "invalid",
    constantSymbol: ks !== "" && !/^\d{1,4}$/.test(ks) && "invalid",
    orderRef: textRule(d.orderRef, { max: 100 }),
    headerNote: textRule(d.headerNote, { max: 2000 }),
    footerNote: textRule(d.footerNote, { max: 2000 }),
    // An imported correction is entered as printed: no reason required.
    correctionReason: isCorrection(d.docType) && textRule(d.correctionReason, { required: !d.imported, max: 500 }),
    number: d.imported && textRule(d.number, { required: true, max: 40 }),
  });
  Object.assign(errors, validateMetadata(d.meta, fieldDefs));
  d.lines.forEach((line, i) => {
    // Advance descriptions are generated by the server.
    if (line.kind === "advance") return;
    const description = textRule(line.description, { required: line.kind !== "subtotal", max: 500 });
    if (description) errors[`lines.${i}.description`] = description;
  });
  return errors;
}

/**
 * `create`: POST treats a null note as omitted and applies the company default, so a note the user
 * cleared goes out as "" (no note); PUT applies no defaults and keeps null.
 */
export function toInput(d: DocumentDraft, fieldDefs: CustomField[] = [], create = false): DocumentInput {
  const note = (v: string) => (create ? v.trim() : nullIfEmpty(v));
  return {
    docType: d.docType,
    direction: "issued",
    contactId: d.contactId,
    issueDate: d.issueDate,
    taxPointDate: d.docType === "proforma" ? null : nullIfEmpty(d.taxPointDate),
    dueDate: d.dueDate,
    currency: d.currency.trim().toUpperCase(),
    exchangeRate: nullIfEmpty(d.exchangeRate)?.replace(",", ".") ?? null,
    locale: d.locale,
    vatMode: d.vatMode,
    bankAccountId: nullIfEmpty(d.bankAccountId),
    paymentMethod: d.paymentMethod,
    variableSymbol: nullIfEmpty(d.variableSymbol),
    constantSymbol: nullIfEmpty(d.constantSymbol),
    orderRef: nullIfEmpty(d.orderRef),
    headerNote: note(d.headerNote),
    footerNote: note(d.footerNote),
    roundingMode: d.roundingMode,
    correctionReason: isCorrection(d.docType) ? nullIfEmpty(d.correctionReason) : null,
    imported: d.imported,
    number: d.imported ? d.number.trim() : null,
    // Natively the link is made by settle / credit note; only an import sets it itself.
    ...(d.imported ? { relatedDocumentId: d.relatedDocumentId } : {}),
    ...toMetadataInput(d.meta, fieldDefs),
    lines: d.lines.map(toWireLine),
  };
}

/**
 * Body for the live totals. Without a manual rate the indicative ČNB rate is
 * used for the CZK preview only — it is never saved, so issue fixes the real one.
 */
export function toComputeRequest(d: DocumentDraft, indicativeRate: string | null = null, documentId?: string): ComputeRequest {
  const { lines, vatMode, currency, exchangeRate, roundingMode, contactId, locale } = toInput(d);
  return {
    lines,
    vatMode,
    currency,
    exchangeRate: exchangeRate ?? indicativeRate,
    roundingMode,
    contactId,
    docType: d.docType,
    locale,
    ...(documentId ? { documentId } : {}),
  };
}
