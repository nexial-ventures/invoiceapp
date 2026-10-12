import type { DocLocale } from "@/api/types";

/** `GET /api/settings/email` — SMTP comes from env only; `replyTo` is the company e-mail. */
export interface EmailSettings {
  configured: boolean;
  /** The operator's kill switch (`INVOICE__EMAIL_DISABLED`): nothing can be sent. */
  disabled: boolean;
  from: string | null;
  replyTo: string | null;
}

/** One locale's effective template; `custom` = an override exists in storage. */
export interface EmailTemplate {
  locale: DocLocale;
  subject: string;
  body: string;
  custom: boolean;
}

export interface TemplatesResponse {
  templates: EmailTemplate[];
}

/** Body of template save / preview, and the rendered preview. */
export interface TemplateText {
  subject: string;
  body: string;
}

export interface AttachmentInfo {
  available: boolean;
  filename: string;
}

/** `GET /api/documents/{id}/email` — the send dialog prefill. */
export interface EmailPrefill {
  configured: boolean;
  disabled: boolean;
  locale: DocLocale;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  body: string;
  attachments: { pdf: AttachmentInfo; isdoc: AttachmentInfo };
}

export interface SendEmailInput {
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  body: string;
  attachPdf: boolean;
  attachIsdoc: boolean;
}

/** One send attempt (`GET /api/documents/{id}/emails`, newest first). */
export interface EmailLogEntry {
  id: string;
  createdAt: string;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  body: string;
  attachments: string[];
  ok: boolean;
  error: string | null;
  messageId: string | null;
}

export type RecipientField = "to" | "cc" | "bcc";
