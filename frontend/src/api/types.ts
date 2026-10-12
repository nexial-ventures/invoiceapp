import { allOf, type Schemas } from "./wire";

/** `GET /api/health` */
export interface HealthResponse {
  status: string;
  version: string;
}

/** Field name (camelCase wire name) → reason code, e.g. `{ ico: "invalid_ico" }`. */
export type FieldErrors = Record<string, string>;

/**
 * Error body returned by every `/api/*` endpoint; `fields` only on 422 `validation`,
 * `detail` only on 502 `pdf_render_failed`.
 */
export interface ApiErrorBody {
  code: string;
  fields?: FieldErrors;
  detail?: string;
}

export type DocLocale = "cs" | "en";

/** `GET /api/ares/{ico}` — a contact draft from the Czech business register. */
export interface AresSubject {
  name: string;
  ico: string;
  dic: string | null;
  street: string;
  city: string;
  zip: string;
  country: string;
}

/** Shared document enums (documents, settings, exports); unions from the generated schema. */
export type Direction = "issued" | "received";
export type PaymentMethod = Schemas["PaymentMethod"];
export const PAYMENT_METHODS = allOf<PaymentMethod>()(["bank_transfer", "cash", "card", "other"]);
/** `half` → nearest 0.50, `unit` → whole units of the document currency (half away from zero). */
export type RoundingMode = Schemas["RoundingMode"];
export const ROUNDING_MODES = allOf<RoundingMode>()(["none", "half", "unit"]);

/** Number-series keys of issued documents, in display order. */
export const ISSUED_SERIES = [
  "invoice",
  "simplified",
  "proforma",
  "credit_note",
  "debit_note",
  "advance_tax_doc",
  "advance_credit_note",
] as const;
/** The series of received documents (1e, 1f-a); `received` is the received invoices'. */
export const RECEIVED_SERIES = [
  "received",
  "received_simplified",
  "received_proforma",
  "received_credit_note",
  "received_debit_note",
  "received_advance_tax_doc",
  "received_advance_credit_note",
] as const;
/** All fourteen number-series keys. */
export type DocType = Schemas["DocType"];
export const DOC_TYPES = allOf<DocType>()([...ISSUED_SERIES, ...RECEIVED_SERIES]);
