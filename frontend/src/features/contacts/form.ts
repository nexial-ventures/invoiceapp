import type { FieldErrors } from "@/api/types";
import { collectErrors, nullIfEmpty, textRule } from "@/lib/formErrors";
import { validateParty, type PartyDraft } from "@/components/form/party";
import type { Contact, ContactInput } from "./types";

/** Form state: every input is a string; converted to the wire shape on submit. */
export interface ContactDraft extends PartyDraft {
  email: string;
  phone: string;
  note: string;
  defaultDueDays: string;
  defaultLocale: "" | "cs" | "en";
  defaultCurrency: string;
  numberCode: string;
}

/** Same rule as the server: the code is printed inside numbers. */
const NUMBER_CODE = /^[A-Za-z0-9/_-]{1,10}$/;

export function toDraft(c?: Contact): ContactDraft {
  return {
    name: c?.name ?? "",
    ico: c?.ico ?? "",
    dic: c?.dic ?? "",
    street: c?.street ?? "",
    city: c?.city ?? "",
    zip: c?.zip ?? "",
    country: c?.country ?? "CZ",
    email: c?.email ?? "",
    phone: c?.phone ?? "",
    note: c?.note ?? "",
    defaultDueDays: c?.defaultDueDays == null ? "" : String(c.defaultDueDays),
    defaultLocale: c?.defaultLocale ?? "",
    defaultCurrency: c?.defaultCurrency ?? "",
    numberCode: c?.numberCode ?? "",
  };
}

export function validateContact(d: ContactDraft): FieldErrors {
  const due = d.defaultDueDays.trim();
  const currency = d.defaultCurrency.trim();
  const code = d.numberCode.trim();
  return {
    ...validateParty(d),
    ...collectErrors({
      email: textRule(d.email, { max: 200 }),
      defaultDueDays: due !== "" && !/^\d{1,3}$/.test(due) ? "invalid" : null,
      defaultCurrency: currency !== "" && !/^[A-Za-z]{3}$/.test(currency) ? "invalid" : null,
      numberCode: code !== "" && !NUMBER_CODE.test(code) ? "invalid" : null,
    }),
  };
}

export function toInput(d: ContactDraft): ContactInput {
  const currency = nullIfEmpty(d.defaultCurrency);
  const due = nullIfEmpty(d.defaultDueDays);
  return {
    name: d.name.trim(),
    ico: nullIfEmpty(d.ico),
    dic: nullIfEmpty(d.dic)?.toUpperCase() ?? null,
    street: d.street.trim(),
    city: d.city.trim(),
    zip: d.zip.trim(),
    country: d.country.trim().toUpperCase(),
    email: nullIfEmpty(d.email),
    phone: nullIfEmpty(d.phone),
    note: nullIfEmpty(d.note),
    defaultDueDays: due === null ? null : Number(due),
    defaultLocale: d.defaultLocale === "" ? null : d.defaultLocale,
    defaultCurrency: currency?.toUpperCase() ?? null,
    // PUT replaces every field: always sent, or saving the form would clear it.
    numberCode: nullIfEmpty(d.numberCode),
  };
}
