import type { DocLocale } from "@/api/types";
import type { DocType } from "@/features/settings/types";

/** Wire types for `/api/documents` (Phases 1b–1f). Decimals are strings, dates `YYYY-MM-DD`. */

export type Direction = "issued" | "received";
export const DOC_STATUSES = ["draft", "issued", "cancelled"] as const;
export type DocStatus = (typeof DOC_STATUSES)[number];
export const PAYMENT_STATES = ["unpaid", "partial", "paid", "overpaid"] as const;
export type PaymentState = (typeof PAYMENT_STATES)[number];
export const VAT_MODES = ["standard", "reverse_charge", "exempt", "non_payer"] as const;
export type VatMode = (typeof VAT_MODES)[number];
export const PAYMENT_METHODS = ["bank_transfer", "cash", "card", "other"] as const;
export type PaymentMethod = (typeof PAYMENT_METHODS)[number];
/** `half` → nearest 0.50, `unit` → whole units of the document currency (half away from zero). */
export const ROUNDING_MODES = ["none", "half", "unit"] as const;
export type RoundingMode = (typeof ROUNDING_MODES)[number];

export interface ItemLine {
  kind: "item";
  description: string;
  quantity: string;
  unit: string | null;
  unitPrice: string;
  discountPct: string;
  vatRate: string;
}
export interface TextLine {
  kind: "text";
  description: string;
}
export interface SubtotalLine {
  kind: "subtotal";
  description: string;
  /** 1-based positions of other lines. */
  refs: number[];
  collapse: boolean;
}
/** Deducts an issued DDPP (or, for non-payers, the proforma itself) on a final invoice. */
export interface AdvanceLine {
  kind: "advance";
  advanceDocumentId: string;
}
export type DocumentLine = ItemLine | TextLine | SubtotalLine | AdvanceLine;

/** Deducted amounts of an advance line per VAT rate (negative). */
export interface AdvanceRecapRow {
  vatRate: string;
  base: string;
  vat: string;
}

/** Server-generated display fields of an advance line. */
export interface AdvanceDisplay {
  description: string;
  base: string;
  recap: AdvanceRecapRow[];
}

/** Response lines carry their position and, for item/subtotal/advance, the computed base. */
export type ComputedLine =
  | (ItemLine & { position: number; base: string })
  | (TextLine & { position: number })
  | (SubtotalLine & { position: number; base: string; vatRate?: string | null })
  | (AdvanceLine & AdvanceDisplay & { position: number });

/**
 * The seven ISDOC document types, in list-tab order; received documents use the same values
 * (told apart by `direction`).
 */
export const DOCUMENT_TYPES = [
  "invoice",
  "simplified",
  "proforma",
  "credit_note",
  "debit_note",
  "advance_tax_doc",
  "advance_credit_note",
] as const;
export type DocumentType = (typeof DOCUMENT_TYPES)[number];

/**
 * Doc types the issued editor can edit: natively an invoice, proforma or simplified document starts
 * empty, corrections come from their original and DDPPs from payments; an import may be any type.
 */
export type EditableDocType = DocumentType;

/** `{ [customField.key]: value }` — text/number (decimal)/date/select strings, bool booleans; null = empty. */
export type CustomFieldValues = Record<string, string | boolean | null>;

/** Category, custom fields and internal note: editable in every status (`PUT …/metadata`). */
export interface MetadataInput {
  categoryId: string | null;
  customFields: CustomFieldValues;
  internalNote: string | null;
}

export interface DocumentInput {
  docType: DocType;
  direction: Direction;
  contactId: string | null;
  issueDate: string;
  taxPointDate: string | null;
  dueDate: string;
  currency: string;
  exchangeRate: string | null;
  locale: DocLocale;
  vatMode: VatMode;
  bankAccountId: string | null;
  paymentMethod: PaymentMethod;
  variableSymbol: string | null;
  constantSymbol: string | null;
  orderRef: string | null;
  headerNote: string | null;
  footerNote: string | null;
  internalNote: string | null;
  roundingMode: RoundingMode;
  /** Credit / debit notes and DDPP corrections; required at issue (not for imported ones). */
  correctionReason: string | null;
  /** Manual import of an existing document: keeps its own `number`, issue renders no PDF. Immutable. */
  imported: boolean;
  /** Required iff imported, else null. */
  number: string | null;
  /** Imported only (DDPP / final invoice → proforma, credit / debit note → invoice or simplified, DDPP correction → DDPP); natively set by settle / corrections. */
  relatedDocumentId?: string | null;
  categoryId: string | null;
  customFields: CustomFieldValues;
  lines: DocumentLine[];
}

/** One row of a received document's VAT recap, as entered from the supplier's document (positive for credit notes). */
export interface VatRecapEntry {
  rate: string;
  base: string;
  vat: string;
}

/** Body of create / PUT for `direction: "received"`: no lines, no draft (saved = recorded). */
export interface ReceivedDocumentInput {
  direction: "received";
  docType: DocumentType;
  contactId: string | null;
  supplierNumber: string;
  issueDate: string;
  /** null for a proforma. */
  taxPointDate: string | null;
  receivedDate: string;
  /** null for a DDPP. */
  dueDate: string | null;
  currency: string;
  exchangeRate: string | null;
  vatMode: VatMode;
  vatRecap: VatRecapEntry[];
  rounding: string;
  payable: string;
  vatDeductible: boolean;
  variableSymbol: string | null;
  constantSymbol: string | null;
  supplierAccount: string | null;
  relatedDocumentId: string | null;
  categoryId: string | null;
  customFields: CustomFieldValues;
  internalNote: string | null;
}

/** The uploaded original PDF of a received / imported document. */
export interface OriginalPdf {
  sha256: string;
  size: number;
  uploadedAt: string;
}

export interface RecapRow {
  vatRate: string;
  base: string;
  vat: string;
  baseCzk: string | null;
  vatCzk: string | null;
}

export interface Totals {
  recap: RecapRow[];
  base: string;
  vat: string;
  total: string;
  rounding: string;
  payable: string;
  totalCzk: string | null;
}

export interface PartySnapshot {
  name: string;
  ico: string | null;
  dic: string | null;
  street: string;
  city: string;
  zip: string;
  country: string;
  registration: string | null;
  vatPayer: boolean | null;
}

export interface BankSnapshot {
  accountNumber: string | null;
  iban: string | null;
  bic: string | null;
}

export interface Document extends Omit<DocumentInput, "lines" | "dueDate" | "number" | "relatedDocumentId"> {
  id: string;
  number: string | null;
  /** null for a received DDPP. */
  dueDate: string | null;
  status: DocStatus;
  paymentState: PaymentState | null;
  overdue: boolean;
  sentAt: string | null;
  cancelledAt: string | null;
  cancelReason: string | null;
  exchangeRateDate: string | null;
  /** `original`: a correction's rate copied from its original document. */
  exchangeRateSource: "cnb" | "manual" | "original" | null;
  supplier: PartySnapshot | null;
  customer: PartySnapshot | null;
  bankSnapshot: BankSnapshot | null;
  lines: ComputedLine[];
  totals: Totals;
  paid: string;
  /** credit / debit note → invoice or simplified, DDPP correction → DDPP, invoice → the proforma it settles, DDPP → proforma. */
  relatedDocumentId: string | null;
  /** DDPP only: the proforma payment it documents. */
  paymentId: string | null;
  /** The document `relatedDocumentId` points at (invoice of a credit note, proforma of a DDPP / final invoice). */
  parent: RelatedDocument | null;
  /** Documents whose `relatedDocumentId` is this one. */
  relatedDocuments: RelatedDocument[];
  /** Proforma only: a non-cancelled invoice settles it. */
  settled: boolean | null;
  /** Issued DDPP only: why a correction cannot be created now; null = allowed (and on every other document). */
  correctionBlock: CorrectionBlock | null;
  /** -1 for credit notes and DDPP corrections: amounts are stored positive, shown negated. */
  sign: Sign;
  /** The archived PDF (written at issue; a DDPP's possibly on first download). Drafts, received, imported: null. */
  pdf: PdfArchive | null;
  /** Received / imported only: the uploaded original. */
  original: OriginalPdf | null;
  // Received documents only (absent / null on issued ones).
  supplierNumber?: string | null;
  receivedDate?: string | null;
  vatRecap?: VatRecapEntry[] | null;
  rounding?: string | null;
  total?: string | null;
  payable?: string | null;
  vatDeductible?: boolean | null;
  supplierAccount?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface PdfArchive {
  sha256: string;
  renderedAt: string;
}

export type Sign = 1 | -1;

export const CORRECTION_BLOCKS = ["advance_settled", "advance_in_use", "fully_corrected"] as const;
export type CorrectionBlock = (typeof CORRECTION_BLOCKS)[number];

export interface RelatedDocument {
  id: string;
  docType: DocType;
  number: string | null;
  status: DocStatus;
  payable: string;
  /** Linked received documents may differ in currency (a CZK DDPP of a EUR proforma); absent = this document's. */
  currency?: string;
}

export interface DocumentSummary {
  id: string;
  docType: DocType;
  direction: Direction;
  number: string | null;
  status: DocStatus;
  paymentState: PaymentState | null;
  overdue: boolean;
  contactId: string | null;
  /** The counterparty: customer of an issued, supplier of a received document. */
  customerName: string | null;
  issueDate: string;
  dueDate: string | null;
  currency: string;
  payable: string;
  paid: string;
  sentAt: string | null;
  sign: Sign;
  relatedDocumentId: string | null;
  supplierNumber?: string | null;
  categoryId?: string | null;
  /** Rendered archive or uploaded original present. */
  hasPdf?: boolean;
  imported?: boolean;
}

export interface DocumentPage {
  items: DocumentSummary[];
  total: number;
}

export interface DocumentListQuery {
  direction?: Direction;
  docType?: DocType;
  status?: DocStatus;
  paymentState?: PaymentState;
  overdue?: boolean;
  contactId?: string;
  categoryId?: string;
  imported?: boolean;
  q?: string;
  from?: string;
  to?: string;
  limit: number;
  offset: number;
}

/**
 * `POST /api/documents/compute`. `contactId`, `docType`, `documentId` (the edited draft) let
 * advance lines validate as on save; `locale` picks the advance description language.
 */
export interface ComputeRequest {
  lines: DocumentLine[];
  vatMode: VatMode;
  currency: string;
  exchangeRate: string | null;
  roundingMode: RoundingMode;
  contactId: string | null;
  docType: EditableDocType;
  locale: DocLocale;
  documentId?: string;
}
export interface ComputeResult {
  lines: ComputedLine[];
  totals: Totals;
}

export interface Payment {
  id: string;
  date: string;
  amount: string;
  note: string | null;
  /** The DDPP this payment created (proforma of a VAT payer). */
  advanceDocumentId: string | null;
  createdAt: string;
}
export interface PaymentInput {
  date: string;
  amount: string;
  note: string | null;
  /** Foreign-currency proforma only; null = ČNB rate for the payment date. */
  exchangeRate?: string | null;
}

/** `GET /api/exchange-rates/{currency}?date=` — `date` is the ČNB publication date used. */
export interface ExchangeRate {
  currency: string;
  date: string;
  rate: string;
}
