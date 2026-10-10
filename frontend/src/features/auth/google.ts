/** `link` is not started by a plain link: the account page gets its start URL from `POST /api/account/google/link`. */
export type GooglePurpose = "login" | "invite";

/**
 * `GET /api/auth/google/start` on the current host. A plain navigation (not fetch): the server
 * redirects to Google and, at the end, back to this host.
 */
export function googleStartUrl(purpose: GooglePurpose, params: { returnTo?: string; invite?: string } = {}): string {
  const query = new URLSearchParams({ purpose });
  if (params.returnTo !== undefined) query.set("return", params.returnTo);
  if (params.invite !== undefined) query.set("invite", params.invite);
  return `/api/auth/google/start?${query.toString()}`;
}

/** `?google_error=` codes of the Google flow (docs/api/google.md). */
export const GOOGLE_ERRORS = [
  "cancelled",
  "expired",
  "invalid",
  "unverified_email",
  "not_allowed",
  "disabled",
  "not_member",
  "mfa_required",
  "already_linked",
  "email_mismatch",
  "unavailable",
] as const;

/** i18n key for a `google_error` query value, null when there is none; unknown codes get a generic text. */
export function googleErrorKey(code: unknown): string | null {
  if (typeof code !== "string" || code === "") return null;
  return (GOOGLE_ERRORS as readonly string[]).includes(code) ? `google.errors.${code}` : "google.errors.unknown";
}
