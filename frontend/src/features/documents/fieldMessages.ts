import type { FieldErrors } from "@/api/types";
import { reasonKey } from "@/lib/formErrors";
import { splitLineErrors } from "./lines";

type Translate = (key: string, params?: Record<string, unknown>) => string;

/** Header fields that have a `documents.fields.*` label. */
export const FIELD_LABELS = [
  "docType",
  "number",
  "contactId",
  "issueDate",
  "taxPointDate",
  "dueDate",
  "currency",
  "exchangeRate",
  "locale",
  "vatMode",
  "bankAccountId",
  "paymentMethod",
  "variableSymbol",
  "constantSymbol",
  "orderRef",
  "headerNote",
  "footerNote",
  "internalNote",
  "roundingMode",
  "correctionReason",
  "lines",
  "date",
  "amount",
  "note",
  "reason",
] as const;

export const LINE_FIELD_LABELS = ["description", "quantity", "unit", "unitPrice", "discountPct", "vatRate", "refs", "kind", "advanceDocumentId"] as const;

const has = (list: readonly string[], v: string) => list.includes(v);

/**
 * 422 field reasons as readable sentences, for places without an input to attach
 * them to (e.g. issuing from the detail view). Line indexes are shown 1-based.
 */
export function describeFieldErrors(fields: FieldErrors, t: Translate): string[] {
  const { header, lines } = splitLineErrors(fields);
  const out = Object.entries(header).map(([field, reason]) => {
    const label = has(FIELD_LABELS, field) ? t(`documents.fields.${field}`) : field;
    return `${label}: ${t(reasonKey(reason))}`;
  });
  for (const [index, lineFields] of Object.entries(lines)) {
    for (const [field, reason] of Object.entries(lineFields)) {
      const label = has(LINE_FIELD_LABELS, field) ? t(`documents.line.${field}`) : field;
      out.push(`${t("documents.line.rowLabel", { n: Number(index) + 1 })}, ${label}: ${t(reasonKey(reason))}`);
    }
  }
  return out;
}
