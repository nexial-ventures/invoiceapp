import { useI18n } from "vue-i18n";
import { useErrorText } from "@/composables/useAction";
import { isApiError } from "@/lib/formErrors";

/** Whole minutes below an hour, else whole hours (rounded up). */
export function waitParams(seconds: number): { key: string; n: number } {
  return seconds < 3600
    ? { key: "errors.waitMinutes", n: Math.max(1, Math.ceil(seconds / 60)) }
    : { key: "errors.waitHours", n: Math.ceil(seconds / 3600) };
}

/**
 * Error text where an e-mail is sent on behalf of the space (document, test, invitation): a 429 there is the
 * hourly / daily e-mail limit, not too many attempts, and says when it frees up.
 */
export function useEmailErrorText(): (err: unknown) => string {
  const { t } = useI18n();
  const errorText = useErrorText();
  return (err) => {
    if (!isApiError(err, 429, "rate_limited")) return errorText(err);
    const secs = err.retryAfter;
    if (secs === null) return t("errors.emailRateLimited");
    const { key, n } = waitParams(secs);
    return t("errors.emailRateLimitedWait", { wait: t(key, { n }) });
  };
}
