import { request, requestBlob } from "@/api/client";
import type {
  ComputeRequest,
  ComputeResult,
  Document,
  DocumentInput,
  DocumentListQuery,
  DocumentPage,
  ExchangeRate,
  MetadataInput,
  Payment,
  PaymentInput,
  ReceivedDocumentInput,
} from "./types";

const BASE = "/api/documents";
const item = (id: string) => `${BASE}/${encodeURIComponent(id)}`;

/** List filters without paging; also the ISDOC bulk export's query. */
export type DocumentFilterQuery = Omit<DocumentListQuery, "limit" | "offset">;

export function filterParams(query: DocumentFilterQuery): URLSearchParams {
  const params = new URLSearchParams();
  const { overdue, imported, q, ...rest } = query;
  for (const [key, value] of Object.entries(rest)) {
    if (value) params.set(key, value);
  }
  if (overdue) params.set("overdue", "true");
  if (imported !== undefined) params.set("imported", String(imported));
  if (q?.trim()) params.set("q", q.trim());
  return params;
}

function listParams({ limit, offset, ...filters }: DocumentListQuery): string {
  const params = filterParams(filters);
  params.set("limit", String(limit));
  params.set("offset", String(offset));
  return params.toString();
}

export const documentsApi = {
  list: (query: DocumentListQuery) => request<DocumentPage>(`${BASE}?${listParams(query)}`),
  get: (id: string) => request<Document>(item(id)),
  create: (input: DocumentInput | ReceivedDocumentInput) => request<Document>(BASE, { method: "POST", body: input }),
  update: (id: string, input: DocumentInput | ReceivedDocumentInput) =>
    request<Document>(item(id), { method: "PUT", body: input }),
  remove: (id: string) => request<void>(item(id), { method: "DELETE" }),
  compute: (body: ComputeRequest, signal?: AbortSignal) =>
    request<ComputeResult>(`${BASE}/compute`, { method: "POST", body, signal }),
  issue: (id: string) => request<Document>(`${item(id)}/issue`, { method: "POST" }),
  cancel: (id: string, reason: string | null) =>
    request<Document>(`${item(id)}/cancel`, { method: "POST", body: reason ? { reason } : {} }),
  markSent: (id: string) => request<Document>(`${item(id)}/mark-sent`, { method: "POST", body: {} }),
  /** Issued → draft, number kept (admin+); `reason` required. */
  unlock: (id: string, reason: string) => request<Document>(`${item(id)}/unlock`, { method: "POST", body: { reason } }),
  /** Category, custom fields and internal note; every status, both directions. */
  setMetadata: (id: string, input: MetadataInput) => request<Document>(`${item(id)}/metadata`, { method: "PUT", body: input }),
  /** Draft final invoice from an issued proforma. */
  settle: (id: string) => request<Document>(`${item(id)}/settle`, { method: "POST" }),
  /** Credit-note draft for an issued invoice / simplified document; on an issued DDPP a DDPP correction. */
  creditNote: (id: string, correctionReason: string | null) =>
    request<Document>(`${item(id)}/credit-note`, { method: "POST", body: correctionReason ? { correctionReason } : {} }),
  /** Debit-note draft (no lines) for an issued invoice / simplified document. */
  debitNote: (id: string, correctionReason: string | null) =>
    request<Document>(`${item(id)}/debit-note`, { method: "POST", body: correctionReason ? { correctionReason } : {} }),
  /** Draft: rendered live with the watermark; issued/cancelled: the archived file; received/imported: the original. */
  pdf: (id: string, download = false) => requestBlob(`${item(id)}/pdf${download ? "?download=1" : ""}`),
  /** Received / imported: upload (or replace) the original PDF, multipart part `file`. */
  uploadOriginal: (id: string, file: File) => {
    const body = new FormData();
    body.append("file", file);
    // The response body is not relied on: the caller reloads the document.
    return request<unknown>(`${item(id)}/original`, { method: "PUT", body });
  },
  removeOriginal: (id: string) => request<void>(`${item(id)}/original`, { method: "DELETE" }),
  payments: (id: string) => request<Payment[]>(`${item(id)}/payments`),
  addPayment: (id: string, input: PaymentInput) =>
    request<Payment>(`${item(id)}/payments`, { method: "POST", body: input }),
  removePayment: (id: string, paymentId: string) =>
    request<void>(`${item(id)}/payments/${encodeURIComponent(paymentId)}`, { method: "DELETE" }),
};

export const exchangeRatesApi = {
  get: (currency: string, date: string) =>
    request<ExchangeRate>(
      `/api/exchange-rates/${encodeURIComponent(currency)}?${new URLSearchParams({ date }).toString()}`,
    ),
};
