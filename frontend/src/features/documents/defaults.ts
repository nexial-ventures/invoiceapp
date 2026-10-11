import type { DocLocale } from "@/api/types";
import type { Company } from "@/features/settings/types";
import type { DocumentDraft } from "./form";
import type { PaymentMethod, RoundingMode } from "./types";

/** Company rounding default for a payment method: cash has its own, every other method shares one. */
export function defaultRounding(company: Company, method: PaymentMethod): RoundingMode {
  return method === "cash" ? company.defaultRoundingCash : company.defaultRoundingOther;
}

/** Company default note for a locale; "" when there is none (draft form state). */
export function defaultNote(company: Company, kind: "header" | "footer", locale: DocLocale): string {
  const notes = kind === "header" ? company.defaultHeaderNote : company.defaultFooterNote;
  return notes[locale] ?? "";
}

/** New payment method; a rounding the user never touched follows the company default for it. */
export function changePaymentMethod(draft: DocumentDraft, method: PaymentMethod, company: Company): DocumentDraft {
  return {
    ...draft,
    paymentMethod: method,
    roundingMode: draft.roundingAuto ? defaultRounding(company, method) : draft.roundingMode,
  };
}

/** A manual pick sticks: later payment method changes no longer re-apply the default. */
export function changeRounding(draft: DocumentDraft, mode: RoundingMode): DocumentDraft {
  return { ...draft, roundingMode: mode, roundingAuto: false };
}

/**
 * New locale; each note still equal to the old locale's default is swapped for the new one.
 * A note that differs was edited by the user (or is a saved draft's own text) and stays.
 */
export function changeLocale(draft: DocumentDraft, locale: DocLocale, company: Company): DocumentDraft {
  if (locale === draft.locale) return draft;
  const swap = (kind: "header" | "footer", current: string) =>
    current === defaultNote(company, kind, draft.locale) ? defaultNote(company, kind, locale) : current;
  return {
    ...draft,
    locale,
    headerNote: swap("header", draft.headerNote),
    footerNote: swap("footer", draft.footerNote),
  };
}
