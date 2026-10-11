import type { DocLocale, FieldErrors } from "@/api/types";
import { collectErrors, nullIfEmpty, textRule } from "@/lib/formErrors";
import { validateParty, type PartyDraft } from "@/components/form/party";
import { LOCALES } from "@/i18n";
import type { PaymentMethod, RoundingMode } from "@/features/documents/types";
import type { Company, LocalizedNote } from "./types";

/** Per-locale note text; "" = none. */
export type NoteDraft = Record<DocLocale, string>;

export interface CompanyDraft extends PartyDraft {
  vatPayer: boolean;
  email: string;
  phone: string;
  web: string;
  registration: string;
  defaultDueDays: string;
  defaultLocale: DocLocale;
  defaultPaymentMethod: PaymentMethod;
  defaultCurrency: string;
  defaultRoundingCash: RoundingMode;
  defaultRoundingOther: RoundingMode;
  defaultHeaderNote: NoteDraft;
  defaultFooterNote: NoteDraft;
}

const toNoteDraft = (n: LocalizedNote | undefined): NoteDraft => ({ cs: n?.cs ?? "", en: n?.en ?? "" });
const toNote = (n: NoteDraft): LocalizedNote => ({ cs: nullIfEmpty(n.cs), en: nullIfEmpty(n.en) });
const NOTE_MAX = 2000;

export function toCompanyDraft(c: Company | null): CompanyDraft {
  return {
    name: c?.name ?? "",
    ico: c?.ico ?? "",
    dic: c?.dic ?? "",
    vatPayer: c?.vatPayer ?? false,
    street: c?.street ?? "",
    city: c?.city ?? "",
    zip: c?.zip ?? "",
    country: c?.country || "CZ",
    email: c?.email ?? "",
    phone: c?.phone ?? "",
    web: c?.web ?? "",
    registration: c?.registration ?? "",
    defaultDueDays: String(c?.defaultDueDays ?? 14),
    defaultLocale: c?.defaultLocale ?? "cs",
    // Fallbacks mirror the server's column defaults.
    defaultPaymentMethod: c?.defaultPaymentMethod ?? "bank_transfer",
    defaultCurrency: c?.defaultCurrency ?? "CZK",
    defaultRoundingCash: c?.defaultRoundingCash ?? "unit",
    defaultRoundingOther: c?.defaultRoundingOther ?? "none",
    defaultHeaderNote: toNoteDraft(c?.defaultHeaderNote),
    defaultFooterNote: toNoteDraft(c?.defaultFooterNote),
  };
}

export function validateCompany(d: CompanyDraft): FieldErrors {
  const due = d.defaultDueDays.trim();
  const notes: Record<string, string | null> = {};
  for (const code of LOCALES) {
    notes[`defaultHeaderNote.${code}`] = textRule(d.defaultHeaderNote[code], { max: NOTE_MAX });
    notes[`defaultFooterNote.${code}`] = textRule(d.defaultFooterNote[code], { max: NOTE_MAX });
  }
  return {
    ...validateParty(d),
    ...collectErrors({
      registration: textRule(d.registration, { max: 300 }),
      defaultDueDays: /^\d{1,3}$/.test(due) && Number(due) <= 365 ? null : "invalid",
      defaultCurrency: !/^[A-Za-z]{3}$/.test(d.defaultCurrency.trim()) && "invalid",
      ...notes,
    }),
  };
}

export function toCompany(d: CompanyDraft): Company {
  return {
    name: d.name.trim(),
    ico: nullIfEmpty(d.ico),
    dic: nullIfEmpty(d.dic)?.toUpperCase() ?? null,
    vatPayer: d.vatPayer,
    street: d.street.trim(),
    city: d.city.trim(),
    zip: d.zip.trim(),
    country: d.country.trim().toUpperCase(),
    email: nullIfEmpty(d.email),
    phone: nullIfEmpty(d.phone),
    web: nullIfEmpty(d.web),
    registration: nullIfEmpty(d.registration),
    defaultDueDays: Number(d.defaultDueDays.trim()),
    defaultLocale: d.defaultLocale,
    defaultPaymentMethod: d.defaultPaymentMethod,
    defaultCurrency: d.defaultCurrency.trim().toUpperCase(),
    defaultRoundingCash: d.defaultRoundingCash,
    defaultRoundingOther: d.defaultRoundingOther,
    defaultHeaderNote: toNote(d.defaultHeaderNote),
    defaultFooterNote: toNote(d.defaultFooterNote),
  };
}
