import type { EmailLogEntry, EmailPrefill, EmailSettings, EmailTemplate } from "./types";

export const emailSettings = (overrides: Partial<EmailSettings> = {}): EmailSettings => ({
  configured: true,
  disabled: false,
  from: "Firma <faktury@firma.cz>",
  replyTo: "me@firma.cz",
  ...overrides,
});

export const prefill = (overrides: Partial<EmailPrefill> = {}): EmailPrefill => ({
  configured: true,
  disabled: false,
  locale: "cs",
  to: ["buyer@acme.cz"],
  cc: [],
  bcc: ["me@firma.cz"],
  subject: "Faktura 20260001 – Firma",
  body: "Dobrý den,\nv příloze…",
  attachments: { pdf: { available: true, filename: "20260001.pdf" }, isdoc: { available: true, filename: "20260001.isdoc" } },
  ...overrides,
});

export const logEntry = (overrides: Partial<EmailLogEntry> = {}): EmailLogEntry => ({
  id: "e1",
  createdAt: "2026-10-08T10:00:00Z",
  to: ["buyer@acme.cz"],
  cc: [],
  bcc: ["me@firma.cz"],
  subject: "Faktura 20260001 – Firma",
  body: "Dobrý den,\nv příloze…",
  attachments: ["20260001.pdf", "20260001.isdoc"],
  ok: true,
  error: null,
  messageId: "<abc@firma.cz>",
  ...overrides,
});

export const template = (locale: "cs" | "en", overrides: Partial<EmailTemplate> = {}): EmailTemplate => ({
  locale,
  subject: "{{ doc.typeLabel }} {{ doc.number }}",
  body: `Body ${locale}`,
  custom: false,
  ...overrides,
});
