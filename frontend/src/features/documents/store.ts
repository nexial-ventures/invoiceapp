import { defineStore } from "pinia";
import { ref } from "vue";
import type { Direction } from "@/api/types";
import { documentsApi, type DocumentFilterQuery } from "./api";
import { DOCUMENT_TYPES, type DocStatus, type Document, type DocumentType, type DocumentInput, type DocumentSummary, type MetadataInput, type Payment, type PaymentInput, type PaymentState, type ReceivedDocumentInput } from "./types";

export const PAGE_SIZE = 50;

/** Doc types the list has a tab for, in tab order: all of them. */
export const LIST_DOC_TYPES = DOCUMENT_TYPES;
export type ListDocType = DocumentType;

export interface InvoiceFilters {
  status: DocStatus | "";
  paymentState: PaymentState | "";
  overdue: boolean;
  q: string;
  from: string;
  to: string;
  categoryId: string;
  /** Issued list only: "" = all, "true" = imported only, "false" = native only. */
  imported: "" | "true" | "false";
}

export const emptyFilters = (): InvoiceFilters => ({
  status: "",
  paymentState: "",
  overdue: false,
  q: "",
  from: "",
  to: "",
  categoryId: "",
  imported: "",
});

/** A document list of one direction; tab, filters + page live here so they survive a trip to the detail view. */
const defineListStore = (id: string, direction: Direction) => defineStore(id, () => {
  const items = ref<DocumentSummary[]>([]);
  const total = ref(0);
  const docType = ref<ListDocType>("invoice");
  const filters = ref<InvoiceFilters>(emptyFilters());
  const offset = ref(0);
  const loading = ref(false);
  // Drops responses that arrive after a newer query was started.
  let seq = 0;

  /** The current tab + filters as a wire query (no paging). */
  function query(): DocumentFilterQuery {
    const f = filters.value;
    return {
      direction,
      docType: docType.value,
      status: f.status || undefined,
      paymentState: f.paymentState || undefined,
      overdue: f.overdue,
      q: f.q,
      from: f.from || undefined,
      to: f.to || undefined,
      categoryId: f.categoryId || undefined,
      imported: f.imported === "" ? undefined : f.imported === "true",
    };
  }

  async function load(): Promise<void> {
    const mine = ++seq;
    loading.value = true;
    try {
      const page = await documentsApi.list({ ...query(), limit: PAGE_SIZE, offset: offset.value });
      if (mine !== seq) return;
      items.value = page.items;
      total.value = page.total;
    } finally {
      if (mine === seq) loading.value = false;
    }
  }

  async function applyFilters(next: InvoiceFilters): Promise<void> {
    filters.value = { ...next };
    offset.value = 0;
    await load();
  }

  async function goTo(newOffset: number): Promise<void> {
    offset.value = Math.max(0, newOffset);
    await load();
  }

  /** Switching the tab keeps the filters but starts at the first page. */
  async function setDocType(next: ListDocType): Promise<void> {
    if (next !== docType.value) {
      docType.value = next;
      offset.value = 0;
      items.value = [];
    }
    await load();
  }

  return { items, total, docType, filters, offset, loading, query, load, applyFilters, goTo, setDocType };
});

export const useInvoiceListStore = defineListStore("documents/invoices", "issued");
export const useReceivedListStore = defineListStore("documents/received", "received");

/** The document open in the detail view, with its payments. Every action stores the server's answer. */
export const useDocumentStore = defineStore("documents/current", () => {
  const doc = ref<Document | null>(null);
  const payments = ref<Payment[]>([]);

  const id = () => {
    if (!doc.value) throw new Error("no document loaded");
    return doc.value.id;
  };

  async function load(docId: string): Promise<void> {
    if (doc.value?.id !== docId) {
      doc.value = null;
      payments.value = [];
    }
    const loaded = await documentsApi.get(docId);
    // A DDPP has no payments of its own.
    payments.value = loaded.status === "draft" || loaded.docType === "advance_tax_doc" ? [] : await documentsApi.payments(docId);
    doc.value = loaded;
  }

  async function save(input: DocumentInput | ReceivedDocumentInput, docId?: string): Promise<Document> {
    doc.value = docId ? await documentsApi.update(docId, input) : await documentsApi.create(input);
    return doc.value;
  }

  async function remove(): Promise<void> {
    await documentsApi.remove(id());
    doc.value = null;
  }

  async function issue(): Promise<void> {
    doc.value = await documentsApi.issue(id());
  }

  async function cancel(reason: string | null): Promise<void> {
    doc.value = await documentsApi.cancel(id(), reason);
  }

  async function markSent(): Promise<void> {
    doc.value = await documentsApi.markSent(id());
  }

  /** Back to a draft: payments stay on the server but are not listed for drafts. */
  async function unlock(reason: string): Promise<void> {
    doc.value = await documentsApi.unlock(id(), reason);
    payments.value = [];
  }

  async function setMetadata(input: MetadataInput): Promise<void> {
    doc.value = await documentsApi.setMetadata(id(), input);
  }

  async function uploadOriginal(file: File): Promise<void> {
    await documentsApi.uploadOriginal(id(), file);
    await load(id());
  }

  async function removeOriginal(): Promise<void> {
    await documentsApi.removeOriginal(id());
    await load(id());
  }

  /** Final-invoice draft settling the loaded proforma. */
  async function settle(): Promise<Document> {
    return documentsApi.settle(id());
  }

  /** Credit-note draft for the loaded invoice / simplified document, or a correction of the loaded DDPP. */
  async function creditNote(reason: string | null): Promise<Document> {
    return documentsApi.creditNote(id(), reason);
  }

  /** Debit-note draft (no lines) for the loaded invoice / simplified document. */
  async function debitNote(reason: string | null): Promise<Document> {
    return documentsApi.debitNote(id(), reason);
  }

  // Payments change `paid` / `paymentState` (and may issue or cancel a DDPP), so the document is reloaded too.
  async function addPayment(input: PaymentInput): Promise<Payment> {
    const payment = await documentsApi.addPayment(id(), input);
    await load(id());
    return payment;
  }

  async function removePayment(paymentId: string): Promise<void> {
    await documentsApi.removePayment(id(), paymentId);
    await load(id());
  }

  return {
    doc,
    payments,
    load,
    save,
    remove,
    issue,
    cancel,
    markSent,
    unlock,
    setMetadata,
    uploadOriginal,
    removeOriginal,
    settle,
    creditNote,
    debitNote,
    addPayment,
    removePayment,
  };
});
