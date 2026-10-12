import { parseCents } from "@/lib/money";
import type { DocumentType, Sign, Totals } from "./types";

/** `sign = −1` documents store positive amounts and are shown negated (credit notes, DDPP corrections). */
export const docSign = (docType: string): Sign => (docType === "credit_note" || docType === "advance_credit_note" ? -1 : 1);

/** Corrections of another document: reason required at issue, a native one keeps the original's customer, currency and rate. */
export const CORRECTION_TYPES = ["credit_note", "debit_note", "advance_credit_note"] as const;
export const isCorrection = (docType: string): boolean => (CORRECTION_TYPES as readonly string[]).includes(docType);

/** Doc types a native document can be started as from "New document"; the rest come from other documents. */
export const NATIVE_NEW_TYPES = ["invoice", "proforma", "simplified"] as const satisfies readonly DocumentType[];
export const isNativeNewType = (docType: string): boolean => (NATIVE_NEW_TYPES as readonly string[]).includes(docType);

/** The fields of a document the correction actions depend on. */
interface Correctable {
  docType: string;
  status: string;
  direction: string;
}

const issuedOwn = (d: Correctable) => d.direction === "issued" && d.status === "issued";

/** An issued invoice / simplified document (imported included) takes credit and debit notes. */
export const canCreditOrDebit = (d: Correctable): boolean =>
  issuedOwn(d) && (d.docType === "invoice" || d.docType === "simplified");

/** An issued DDPP takes a correction (the server still refuses one deducted by an invoice). */
export const canCorrectDdpp = (d: Correctable): boolean => issuedOwn(d) && d.docType === "advance_tax_doc";

/** Above this total in CZK a simplified tax document is not allowed by law; the UI only warns. */
export const SIMPLIFIED_LIMIT_CZK = 10000;

/** True when a simplified document's total in CZK (`totalCzk`, or `total` for CZK) exceeds the limit. */
export function simplifiedOverLimit(docType: string, currency: string, totals: Pick<Totals, "total" | "totalCzk"> | null | undefined): boolean {
  if (docType !== "simplified" || !totals) return false;
  const czk = currency === "CZK" ? totals.total : totals.totalCzk;
  return (parseCents(czk) ?? 0n) > BigInt(SIMPLIFIED_LIMIT_CZK) * 100n;
}

/**
 * Doc types a document may link to via `relatedDocumentId`: DDPP / final invoice → proforma,
 * credit / debit note → invoice or simplified, DDPP correction → DDPP.
 */
export function relatedTargetTypes(docType: string): DocumentType[] {
  switch (docType) {
    case "invoice":
    case "advance_tax_doc":
      return ["proforma"];
    case "credit_note":
    case "debit_note":
      return ["invoice", "simplified"];
    case "advance_credit_note":
      return ["advance_tax_doc"];
    default:
      return [];
  }
}
