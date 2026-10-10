import { ApiError } from "@/api/client";
import type { FieldErrors } from "@/api/types";

export const REASON_CODES = [
  "required",
  "invalid",
  "too_long",
  "duplicate",
  "invalid_ico",
  "invalid_pattern",
  "below_issued",
  "exceeds_original",
  "mixed_vat",
  "unknown",
  "inactive",
  "template_invalid",
  "too_short",
  "reserved",
  "taken",
  "mismatch",
  "too_high",
  "wrong_password",
  "already_member",
  "last_owner",
  "wrong_code",
  "expired",
  // A password-confirmed action by an account that has none (signed up through Google).
  "not_set",
] as const;

/** Field errors of a 422 `validation` response, or null for any other failure. */
export function fieldErrorsOf(err: unknown): FieldErrors | null {
  if (err instanceof ApiError && err.status === 422 && Object.keys(err.fields).length > 0) {
    return { ...err.fields };
  }
  return null;
}

/** i18n key for a reason code; unknown codes degrade to the generic "invalid". */
export function reasonKey(code: string): string {
  return (REASON_CODES as readonly string[]).includes(code)
    ? `validation.${code}`
    : "validation.invalid";
}

/** Statuses a reverse proxy / the Vite dev proxy returns when the backend is down. */
const UNREACHABLE = new Set([0, 502, 503, 504]);

const KNOWN_CODES: Record<string, string> = {
  not_found: "errors.notFound",
  conflict: "errors.conflict",
  validation: "errors.validation",
  ares_not_found: "errors.aresNotFound",
  ares_unavailable: "errors.aresUnavailable",
  document_locked: "errors.documentLocked",
  invalid_state: "errors.invalidState",
  cnb_unavailable: "errors.cnbUnavailable",
  advance_settled: "errors.advanceSettled",
  advance_in_use: "errors.advanceInUse",
  // 409 on cancelling a debit note (the 422 field reason of the same name is `validation.exceeds_original`).
  exceeds_original: "errors.exceedsOriginal",
  catalog_item_in_use: "errors.catalogItemInUse",
  pdf_unavailable: "errors.pdfUnavailable",
  pdf_render_failed: "errors.pdfRenderFailed",
  pdf_missing: "errors.pdfMissing",
  storage_unavailable: "errors.storageUnavailable",
  too_large: "errors.tooLarge",
  category_in_use: "errors.categoryInUse",
  number_taken: "errors.numberTaken",
  // 502 with the SMTP response as `detail`; must win over the generic "unreachable" for 502.
  smtp_failed: "errors.smtpFailed",
  smtp_not_configured: "errors.smtpNotConfigured",
  template_invalid: "errors.templateInvalid",
  forbidden: "errors.forbidden",
  csrf: "errors.csrf",
  rate_limited: "errors.rateLimited",
  email_unverified: "errors.emailUnverified",
  invalid_credentials: "errors.invalidCredentials",
  // 409 on removing / the leave of the last owner (the 422 field reason is `validation.last_owner`).
  last_owner: "errors.lastOwner",
  mfa_invalid: "errors.mfaInvalid",
  mfa_required: "errors.mfaRequired",
  mfa_enabled: "errors.mfaEnabled",
};

/** The error is this API answer (`status` + `code`). */
export function isApiError(err: unknown, status: number, code: string): err is ApiError {
  return err instanceof ApiError && err.status === status && err.code === code;
}

/** i18n key + params for a non-field error message. */
export function errorMessageKey(err: unknown): { key: string; params?: Record<string, string> } {
  if (err instanceof ApiError) {
    const known = KNOWN_CODES[err.code];
    if (known) return { key: known };
    if (UNREACHABLE.has(err.status)) return { key: "errors.unreachable" };
    return { key: "errors.unexpected", params: { code: err.code } };
  }
  return { key: "errors.unexpected", params: { code: "unknown" } };
}

/** Diagnostic text the server attached to the error (`pdf_render_failed`, `smtp_failed`, `template_invalid`), else null. */
export function errorDetailOf(err: unknown): string | null {
  return err instanceof ApiError ? err.detail : null;
}

/** Builds field errors from simple predicate checks: `{ field: reason | null }`. */
export function collectErrors(checks: Record<string, string | null | false>): FieldErrors {
  const out: FieldErrors = {};
  for (const [field, reason] of Object.entries(checks)) {
    if (reason) out[field] = reason;
  }
  return out;
}

/** Common rule for a text field: required + max length. */
export function textRule(value: string, opts: { required?: boolean; max?: number }): string | null {
  const trimmed = value.trim();
  if (opts.required && trimmed === "") return "required";
  if (opts.max !== undefined && trimmed.length > opts.max) return "too_long";
  return null;
}

/** "" → null for nullable wire strings. */
export function nullIfEmpty(value: string): string | null {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}
