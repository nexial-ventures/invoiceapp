import type { Direction } from "@/api/types";
import type { DocumentType } from "@/features/documents/types";

/**
 * Wire types shared by the preview → confirm imports (ISDOC: docs/api/isdoc.md, CSV / XLSX: docs/api/csv.md).
 * Decimals are strings, dates `YYYY-MM-DD`.
 */

export type PreviewStatus = "ok" | "duplicate" | "error";
export type MatchKind = "existing" | "new";

export interface PreviewEntry {
  /** Unique per response; echoed back in `selected`. ISDOC: the path inside the upload, CSV: `row:{n}`. */
  key: string;
  status: PreviewStatus;
  error: string | null;
  warnings: string[];
  direction: Direction | null;
  docType: DocumentType | null;
  number: string | null;
  counterparty: { name: string; ico: string | null } | null;
  contactMatch: MatchKind | null;
  issueDate: string | null;
  taxPointDate: string | null;
  dueDate: string | null;
  currency: string | null;
  total: string | null;
  hasPdf: boolean;
  relatedNumber: string | null;
  relatedFound: boolean;
}

export type ResultStatus = "imported" | "skipped" | "failed";

export interface ConfirmResult {
  key: string;
  status: ResultStatus;
  documentId: string | null;
  error: string | null;
}

export interface ConfirmResponse {
  results: ConfirmResult[];
}
