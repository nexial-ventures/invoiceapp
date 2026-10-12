import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { i18n } from "@/i18n";
import type { Role } from "@/features/spaces/types";
import { mockFetchRoutes, signIn } from "@/test-utils";
import type { DocumentSummary } from "../types";
import InvoicesListView from "./InvoicesListView.vue";

const summary = (overrides: Partial<DocumentSummary>): DocumentSummary => ({
  id: "d1",
  docType: "invoice",
  direction: "issued",
  number: "20260001",
  status: "issued",
  paymentState: "unpaid",
  overdue: true,
  contactId: "c1",
  customerName: "Acme",
  issueDate: "2026-09-01",
  dueDate: "2026-09-15",
  currency: "EUR",
  payable: "100.5",
  paid: "0",
  sentAt: null,
  sign: 1,
  relatedDocumentId: null,
  supplierNumber: null,
  categoryId: null,
  hasPdf: true,
  imported: false,
  ...overrides,
});

async function mountList(path = "/invoices", role: Role = "owner") {
  const pinia = createPinia();
  setActivePinia(pinia);
  signIn(role);
  const stub = { template: "<div />" };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/invoices", name: "invoices", component: InvoicesListView },
      { path: "/invoices/new", name: "invoice-new", component: stub },
      { path: "/import/isdoc", name: "isdoc-import", component: stub },
      { path: "/import/csv", name: "csv-import", component: stub },
      { path: "/invoices/:id", name: "invoice-detail", component: stub },
    ],
  });
  await router.push(path);
  const w = mount({ template: "<RouterView />" }, { global: { plugins: [pinia, i18n, router] } });
  await flushPromises();
  return w;
}

describe("InvoicesListView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("hides creating and importing from an accountant but keeps the exports", async () => {
    mockFetchRoutes({ "GET /api/documents": { items: [], total: 0 } });
    const w = await mountList("/invoices", "accountant");
    for (const test of ["new-document", "import-isdoc", "import-csv", "import-document"]) {
      expect(w.find(`[data-test="${test}"]`).exists(), test).toBe(false);
    }
    expect(w.find('[data-test="accountant-export"]').exists()).toBe(true);
  });

  it("renders rows with badges and formatted amounts", async () => {
    mockFetchRoutes({ "GET /api/documents": { items: [summary({}), summary({ id: "d2", number: null, status: "draft", paymentState: null, overdue: false, currency: "CZK" })], total: 2 } });
    const w = await mountList();
    const rows = w.findAll('[data-test="invoice-row"]');
    expect(rows).toHaveLength(2);
    expect(rows[0]!.text()).toContain("€100.50");
    expect(rows[0]!.find('[data-test="overdue-badge"]').exists()).toBe(true);
    expect(rows[1]!.text()).toContain("Draft");
  });

  it("sends filters; search is debounced and resets paging", async () => {
    const fetch = mockFetchRoutes({ "GET /api/documents": { items: [], total: 0 } });
    const w = await mountList();
    expect(String(fetch.mock.calls[0]?.[0])).toBe("/api/documents?direction=issued&docType=invoice&limit=50&offset=0");

    await w.find("#filter-status").setValue("issued");
    await w.find("#filter-overdue").setValue(true);
    await flushPromises();
    expect(String(fetch.mock.calls.at(-1)?.[0])).toBe("/api/documents?direction=issued&docType=invoice&status=issued&overdue=true&limit=50&offset=0");

    vi.useFakeTimers();
    const before = fetch.mock.calls.length;
    await w.find("#invoice-search").setValue("acme");
    expect(fetch.mock.calls.length).toBe(before);
    await vi.advanceTimersByTimeAsync(300);
    expect(String(fetch.mock.calls.at(-1)?.[0])).toContain("q=acme");
  });

  it("doc-type tabs drive the docType filter and the new-document button", async () => {
    const fetch = mockFetchRoutes({ "GET /api/documents": { items: [], total: 0 } });
    const w = await mountList();
    expect(w.find('[data-test="new-document"]').text()).toBe("New invoice");
    expect(w.find('[data-test="tab-invoice"]').attributes("aria-current")).toBe("page");

    await w.find('[data-test="tab-proforma"]').trigger("click");
    await flushPromises();
    expect(String(fetch.mock.calls.at(-1)?.[0])).toBe("/api/documents?direction=issued&docType=proforma&limit=50&offset=0");
    expect(w.find("h1").text()).toBe("Proforma invoices");
    expect(w.find('[data-test="new-document"]').text()).toBe("New proforma invoice");
    expect(w.find('[data-test="new-document"]').attributes("href")).toBe("/invoices/new?docType=proforma");

    await w.find('[data-test="tab-advance_tax_doc"]').trigger("click");
    await flushPromises();
    expect(String(fetch.mock.calls.at(-1)?.[0])).toContain("docType=advance_tax_doc");
    expect(w.find('[data-test="new-document"]').exists()).toBe(false);
  });

  it("has a tab per doc type; simplified starts natively, corrections only via import", async () => {
    mockFetchRoutes({ "GET /api/documents": { items: [], total: 0 } });
    const w = await mountList();
    expect(w.findAll('[data-test^="tab-"]').map((t) => t.text())).toEqual([
      "Invoices",
      "Simplified documents",
      "Proforma invoices",
      "Credit notes",
      "Debit notes",
      "Advance payment tax documents",
      "Advance payment corrections",
    ]);

    await w.find('[data-test="tab-simplified"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="new-document"]').attributes("href")).toBe("/invoices/new?docType=simplified");

    for (const type of ["debit_note", "advance_credit_note"]) {
      await w.find(`[data-test="tab-${type}"]`).trigger("click");
      await flushPromises();
      expect(w.find('[data-test="new-document"]').exists()).toBe(false);
      expect(w.find('[data-test="import-document"]').attributes("href")).toBe(`/invoices/new?docType=${type}&imported=1`);
    }
  });

  it("credit notes show negated amounts and a badge", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents": { items: [summary({ docType: "credit_note", sign: -1, currency: "CZK", payable: "1210.00", overdue: false })], total: 1 },
    });
    const w = await mountList("/invoices?type=credit_note");
    expect(String(fetch.mock.calls[0]?.[0])).toContain("docType=credit_note");
    const row = w.find('[data-test="invoice-row"]');
    expect(row.text()).toMatch(/-CZK\s1,210\.00/);
    expect(row.find('[data-test="credit-note-badge"]').exists()).toBe(true);
  });

  it("imported / category filters, the imported badge, the no-PDF hint and the category name", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents": {
        items: [summary({ imported: true, hasPdf: false, categoryId: "k1" }), summary({ id: "d2", hasPdf: false })],
        total: 2,
      },
      "GET /api/settings/categories": [
        { id: "k1", name: "Consulting", kind: "income", active: true, position: 0 },
        { id: "k2", name: "Software", kind: "expense", active: true, position: 0 },
      ],
    });
    const w = await mountList();
    const rows = w.findAll('[data-test="invoice-row"]');
    expect(rows[0]!.find('[data-test="imported-badge"]').exists()).toBe(true);
    expect(rows[0]!.find('[data-test="no-pdf"]').text()).toBe("no PDF");
    expect(rows[0]!.find('[data-test="row-category"]').text()).toBe("Consulting");
    // A native document without an archive (e.g. a draft) gets no hint.
    expect(rows[1]!.find('[data-test="no-pdf"]').exists()).toBe(false);
    expect(rows[1]!.find('[data-test="imported-badge"]').exists()).toBe(false);
    // Only income categories are offered on the issued list.
    expect(w.findAll("#filter-category option").map((o) => o.text())).toEqual(["All categories", "Consulting"]);

    await w.find("#filter-imported").setValue("true");
    await flushPromises();
    expect(String(fetch.mock.calls.at(-1)?.[0])).toBe("/api/documents?direction=issued&docType=invoice&imported=true&limit=50&offset=0");
    await w.find("#filter-category").setValue("k1");
    await flushPromises();
    expect(String(fetch.mock.calls.at(-1)?.[0])).toBe("/api/documents?direction=issued&docType=invoice&categoryId=k1&imported=true&limit=50&offset=0");
    expect(w.find('[data-test="import-document"]').attributes("href")).toBe("/invoices/new?docType=invoice&imported=1");
  });

  it("links the ISDOC import and exports the current tab + filters", async () => {
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: vi.fn(() => "blob:x"), revokeObjectURL: vi.fn() }));
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    const fetch = mockFetchRoutes({
      "GET /api/documents": { items: [], total: 0 },
      "GET /api/settings/categories": [],
      "GET /api/documents/isdoc": () => new Response("PK", { status: 200, headers: { "Content-Type": "application/zip" } }),
    });
    const w = await mountList("/invoices?type=proforma");
    expect(w.find('[data-test="import-isdoc"]').attributes("href")).toBe("/import/isdoc");
    expect(w.find('[data-test="import-csv"]').attributes("href")).toBe("/import/csv");
    await w.find('[data-test="isdoc-export"]').trigger("click");
    await flushPromises();
    const url = String(fetch.mock.calls.find(([u]) => String(u).startsWith("/api/documents/isdoc"))?.[0]);
    expect(url).toBe("/api/documents/isdoc?direction=issued&docType=proforma");
    expect(click).toHaveBeenCalled();
    click.mockRestore();
  });
});
