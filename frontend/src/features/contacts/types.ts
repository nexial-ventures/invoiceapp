import type { DocLocale } from "@/api/types";
import type { Wire } from "@/api/wire";

/** `/api/contacts` — one address book for customers and suppliers. */
export interface Contact {
  id: string;
  name: string;
  ico: string | null;
  dic: string | null;
  street: string;
  city: string;
  zip: string;
  country: string;
  email: string | null;
  phone: string | null;
  note: string | null;
  defaultDueDays: number | null;
  defaultLocale: DocLocale | null;
  defaultCurrency: string | null;
  /** The customer's `{C}` in client number patterns (`[A-Za-z0-9/_-]{1,10}`, unique case-insensitively); null = global series. */
  numberCode: string | null;
  createdAt: string;
  updatedAt: string;
}

export type ContactInput = Omit<Contact, "id" | "createdAt" | "updatedAt">;

export interface ContactListQuery {
  q?: string;
  limit: number;
  offset: number;
}

/** `GET /api/contacts` */
export interface ContactPage {
  items: Contact[];
  total: number;
}

/**
 * `GET /api/contacts/{id}/number-counters` — the customer's client sequence per issued series with a client
 * pattern; `nextNumberPreview` null without a `numberCode`.
 */
export type ContactNumberSeries = Wire<"ContactNumberSeries">;
