import type { Direction, DocLocale, DocType, PaymentMethod, RoundingMode } from "@/api/types";
import { allOf, type Narrow, type Schemas, type Wire } from "@/api/wire";

/**
 * Wire types for `/api/documents` (Phases 1b–1f). Decimals are strings, dates `YYYY-MM-DD`.
 * Responses alias the generated schema (`@/api/wire`); request bodies are hand-written.
 */

export type DocStatus = Schemas["Status"];
export const DOC_STATUSES = allOf<DocStatus>()(["draft", "issued", "cancelled"]);
export type PaymentState = Schemas["PaymentState"];
export const PAYMENT_STATES = allOf<PaymentState>()(["unpaid", "partial", "paid", "overpaid"]);
export const VAT_MODES = ["standard", "reverse_charge", "exempt", "non_payer"] as const;
export type VatMode = (typeof VAT_MODES)[number];

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
export type AdvanceRecapRow = Wire<"AdvanceRecap">;

/** Server-generated display fields of an advance line. */
export interface AdvanceDisplay {
  description: string;
  base: string;
  recap: AdvanceRecapRow[];
}

/** Response lines carry their position and, for item/subtotal/advance, the computed base. */
export type ComputedLine = Wire<"Line">;

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
  /**
   * Imported: its own number, required. Native: omitted = keep the draft's number, null = none (allocated at
   * issue), a string = a manual number (`[A-Za-z0-9/_-]`, ≤ 50; docs/api/numbering.md).
   */
  number?: string | null;
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
export type OriginalPdf = Wire<"OriginalPdf">;
export type Totals = Wire<"Totals">;
export type PartySnapshot = Wire<"PartySnapshot">;
export type BankSnapshot = Wire<"BankSnapshot">;

/**
 * `relatedDocumentId`: credit / debit note → invoice or simplified, DDPP correction → DDPP,
 * invoice → the proforma it settles, DDPP → proforma; `parent` is that document, `relatedDocuments`
 * those pointing here. `pdf`: the archive (drafts, received, imported: null); `original`: the
 * uploaded PDF of a received / imported document. Received-only fields are null on issued ones.
 */
export type Document = Narrow<
  Wire<"Document">,
  {
    docType: DocType;
    direction: Direction;
    locale: DocLocale;
    vatMode: VatMode;
    paymentMethod: PaymentMethod;
    /** `original`: a correction's rate copied from its original document. */
    exchangeRateSource: "cnb" | "manual" | "original" | null;
    /** Issued DDPP only: why a correction cannot be created now; null = allowed (and on every other document). */
    correctionBlock: CorrectionBlock | null;
    /** -1 for credit notes and DDPP corrections: amounts are stored positive, shown negated. */
    sign: Sign;
    customFields: CustomFieldValues;
    parent: RelatedDocument | null;
    relatedDocuments: RelatedDocument[];
  }
>;

export type PdfArchive = Wire<"PdfArchive">;

export type Sign = 1 | -1;

export const CORRECTION_BLOCKS = ["advance_settled", "advance_in_use", "fully_corrected"] as const;
export type CorrectionBlock = (typeof CORRECTION_BLOCKS)[number];

/** Linked received documents may differ in currency (a CZK DDPP of a EUR proforma). */
export type RelatedDocument = Narrow<Wire<"RelatedDocument">, { docType: DocType }>;

/** `customerName`: the counterparty (customer of an issued, supplier of a received document); `hasPdf`: archive or original present. */
export type DocumentSummary = Narrow<Wire<"DocumentSummary">, { docType: DocType; direction: Direction; sign: Sign }>;

export type DocumentPage = Narrow<Wire<"DocumentList">, { items: DocumentSummary[] }>;

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
export type ComputeResult = Wire<"Computed">;

/** `advanceDocumentId`: the DDPP this payment created (proforma of a VAT payer). */
export type Payment = Wire<"Payment">;
export interface PaymentInput {
  date: string;
  amount: string;
  note: string | null;
  /** Foreign-currency proforma only; null = ČNB rate for the payment date. */
  exchangeRate?: string | null;
}

/** `GET /api/exchange-rates/{currency}?date=` — `date` is the ČNB publication date used. */
export type ExchangeRate = Wire<"ExchangeRate">;

/** `GET /api/documents/number-preview` — what an issue would allocate now. */
export type NumberPreview = Schemas["NumberPreview"];
