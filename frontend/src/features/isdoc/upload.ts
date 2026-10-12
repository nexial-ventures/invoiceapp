import { ApiError } from "@/api/client";
import { hasExtension } from "@/features/imports/upload";

export const ACCEPTED_EXTENSIONS = [".isdoc", ".isdocx", ".zip"] as const;
export const ACCEPT_ATTR = ACCEPTED_EXTENSIONS.join(",");

const accepted = (file: File) => hasExtension(file, ACCEPTED_EXTENSIONS);

/** Splits picked / dropped files into the ones to upload and the names of the ignored rest. */
export function partitionFiles(files: File[]): { files: File[]; ignored: string[] } {
  return { files: files.filter(accepted), ignored: files.filter((f) => !accepted(f)).map((f) => f.name) };
}

export const ENTRY_ERRORS = [
  "invalid_xml",
  "unsupported_version",
  "unsupported_type",
  "foreign",
  "ambiguous",
  "missing_field",
  "invalid_amount",
  "unsupported_currency",
  "invalid_archive",
  "too_deep",
  "duplicate",
  "number_taken",
  "rate_unavailable",
  "storage_unavailable",
  "quota_exceeded",
  "internal",
] as const;

export const WARNINGS = ["related_not_found", "pdf_skipped", "rate_from_cnb", "contact_created"] as const;

/** i18n key for a per-entry error code (preview or confirm); unknown codes get the generic text with the code. */
export const entryErrorKey = (code: string) =>
  (ENTRY_ERRORS as readonly string[]).includes(code) ? `isdoc.entryError.${code}` : "isdoc.entryError.unknown";

export const warningKey = (code: string) =>
  (WARNINGS as readonly string[]).includes(code) ? `isdoc.warning.${code}` : "isdoc.warning.unknown";

/** i18n key for the upload limits (preview / confirm), or null for any other failure. */
export function uploadErrorKey(err: unknown): string | null {
  if (!(err instanceof ApiError)) return null;
  if (err.status === 413) return "isdoc.upload.tooLarge";
  if (err.status === 422 && err.fields.files === "too_many") return "isdoc.upload.tooMany";
  if (err.status === 422 && err.fields.files === "too_large") return "isdoc.upload.unpackedTooLarge";
  // The category was deactivated or deleted after the page loaded it.
  if (err.status === 422 && err.fields.categoryId) return "isdoc.upload.category";
  return null;
}

/** i18n key for the bulk export's 422 (more than 1000 matching documents), else null. */
export const exportErrorKey = (err: unknown): string | null =>
  err instanceof ApiError && err.status === 422 && err.fields.filter === "too_many" ? "isdoc.export.tooMany" : null;
