import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { i18n } from "@/i18n";
import { document } from "@/features/documents/testData";
import type { DocumentSummary } from "@/features/documents/types";
import type { CustomField } from "@/features/settings/types";
import { calls, mockFetchRoutes, reply, signIn } from "@/test-utils";
import ReceivedDetailView from "./ReceivedDetailView.vue";
import ReceivedListView from "./ReceivedListView.vue";

const summary = (overrides: Partial<DocumentSummary> = {}): DocumentSummary => ({
  id: "r1",
  docType: "invoice",
  direction: "received",
  number: "P20260001",
  status: "issued",
  paymentState: "unpaid",
  overdue: false,
  contactId: "c1",
  customerName: "Supplier s.r.o.",
  issueDate: "2026-10-01",
  dueDate: "2026-10-15",
  currency: "CZK",
  payable: "121.00",
  paid: "0",
  sentAt: null,
  sign: 1,
  relatedDocumentId: null,
  supplierNumber: "FV-77",
  categoryId: "k1",
  hasPdf: false,
  imported: false,
  ...overrides,
});

const categories = [
  { id: "k1", name: "Software", kind: "expense", active: true, position: 0 },
  { id: "k2", name: "Sales", kind: "income", active: true, position: 0 },
];

async function mountAt(path: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  signIn("owner");
  const stub = { template: "<div />" };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/received", name: "received", component: ReceivedListView },
      { path: "/received/new", name: "received-new", component: stub },
      { path: "/import/isdoc", name: "isdoc-import", component: stub },
      { path: "/import/csv", name: "csv-import", component: stub },
      { path: "/received/:id", name: "received-detail", component: ReceivedDetailView },
      { path: "/received/:id/edit", name: "received-edit", component: stub },
      { path: "/invoices/:id", name: "invoice-detail", component: stub },
    ],
  });
  await router.push(path);
  const w = mount({ template: "<RouterView />" }, { global: { plugins: [pinia, i18n, router] } });
  await flushPromises();
  return { w, router };
}

describe("ReceivedListView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("lists received documents with supplier number, category, no-PDF hint and the recorded status", async () => {
    const fetch = mockFetchRoutes({ "GET /api/documents": { items: [summary()], total: 1 }, "GET /api/settings/categories": categories });
    const { w } = await mountAt("/received");
    expect(String(fetch.mock.calls[0]?.[0])).toBe("/api/documents?direction=received&docType=invoice&limit=50&offset=0");
    const row = w.find('[data-test="received-row"]');
    expect(row.text()).toContain("FV-77");
    expect(row.find('[data-test="row-category"]').text()).toBe("Software");
    expect(row.find('[data-test="no-pdf"]').exists()).toBe(true);
    expect(row.find('[data-test="status-badge"]').text()).toBe("Recorded");
    // Received lists have no imported filter; only expense categories are offered.
    expect(w.find("#filter-imported").exists()).toBe(false);
    expect(w.findAll("#filter-category option").map((o) => o.text())).toEqual(["All categories", "Software"]);

    await w.find("#filter-category").setValue("k1");
    await flushPromises();
    expect(String(fetch.mock.calls.at(-1)?.[0])).toBe("/api/documents?direction=received&docType=invoice&categoryId=k1&limit=50&offset=0");
  });

  it("tabs switch the doc type and the new button", async () => {
    const fetch = mockFetchRoutes({ "GET /api/documents": { items: [], total: 0 }, "GET /api/settings/categories": [] });
    const { w } = await mountAt("/received?type=credit_note");
    expect(String(fetch.mock.calls[0]?.[0])).toContain("docType=credit_note");
    expect(w.find("h1").text()).toBe("Received credit notes");
    expect(w.find('[data-test="new-document"]').attributes("href")).toBe("/received/new?docType=credit_note");
    expect(w.find('[data-test="tab-proforma"]').attributes("href")).toBe("/received?type=proforma");
    expect(w.find('[data-test="import-isdoc"]').attributes("href")).toBe("/import/isdoc?from=received");
    expect(w.find('[data-test="import-csv"]').attributes("href")).toBe("/import/csv?from=received");
  });

  it("records any of the seven types, e.g. a debit note", async () => {
    mockFetchRoutes({ "GET /api/documents": { items: [], total: 0 }, "GET /api/settings/categories": [] });
    const { w } = await mountAt("/received?type=debit_note");
    expect(w.findAll('[data-test^="tab-"]')).toHaveLength(7);
    expect(w.find("h1").text()).toBe("Received debit notes");
    expect(w.find('[data-test="new-document"]').attributes("href")).toBe("/received/new?docType=debit_note");
  });
});

describe("ReceivedDetailView", () => {
  const poField: CustomField = { id: "f1", key: "po", label: "PO", type: "text", options: [], appliesTo: "both", required: true, active: true, position: 0 };
  const stored = document({
    id: "r1",
    direction: "received",
    docType: "credit_note",
    sign: -1,
    status: "issued",
    paymentState: "unpaid",
    number: "PD20260001",
    supplierNumber: "DOB-1",
    receivedDate: "2026-10-02",
    vatDeductible: true,
    customFields: { po: "1" },
  });

  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("shows the recorded document with negated totals, payments, original PDF and metadata", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents/r1": stored,
      "GET /api/documents/r1/payments": [],
      "GET /api/settings/categories": categories,
      "GET /api/settings/custom-fields": [poField],
      "PUT /api/documents/r1/metadata": (body: unknown) => ({ ...stored, ...(body as object) }),
    });
    const { w } = await mountAt("/received/r1");
    expect(w.find("h1").text()).toBe("Received credit note PD20260001");
    expect(w.find('[data-test="info-supplierNumber"]').text()).toBe("DOB-1");
    expect(w.find('[data-test="info-roundingMode"]').text()).toBe("No rounding");
    expect(w.find('[data-test="payable"]').text()).toMatch(/-CZK\s1,210\.00/);
    expect(w.find('[data-test="payments-panel"]').exists()).toBe(true);
    expect(w.find('[data-test="original-missing"]').exists()).toBe(true);

    await w.find("#meta-categoryId").setValue("k1");
    await w.find("#meta-cf-po").setValue("");
    await w.find('[data-test="metadata-card"]').trigger("submit");
    expect(w.find("#meta-cf-po-error").text()).toBe("This field is required.");
    await w.find("#meta-cf-po").setValue("PO-9");
    await w.find('[data-test="metadata-card"]').trigger("submit");
    await flushPromises();
    const put = fetch.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({ categoryId: "k1", customFields: { po: "PO-9" }, internalNote: null });
  });

  it("shows ISDOC-imported lines read-only, and nothing when there are none", async () => {
    const routesFor = (doc: unknown) => ({ "GET /api/documents/r1": doc, "GET /api/documents/r1/payments": [], "GET /api/settings/categories": [], "GET /api/settings/custom-fields": [] });
    mockFetchRoutes(routesFor(stored));
    const { w } = await mountAt("/received/r1");
    const section = w.find('[data-test="received-lines"]');
    expect(section.find("h2").text()).toBe("Document lines");
    expect(section.find('[data-test="detail-line-1"]').exists()).toBe(true);
    expect(section.find("input").exists()).toBe(false);

    mockFetchRoutes(routesFor({ ...stored, lines: [] }));
    const { w: empty } = await mountAt("/received/r1");
    expect(empty.find('[data-test="received-lines"]').exists()).toBe(false);
  });

  it("deletes after confirmation and returns to the list tab", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents/r1": stored,
      "GET /api/documents/r1/payments": [],
      "GET /api/settings/categories": [],
      "GET /api/settings/custom-fields": [],
      "DELETE /api/documents/r1": reply(204),
      "GET /api/documents": { items: [], total: 0 },
    });
    vi.stubGlobal("confirm", vi.fn(() => true));
    const { w, router } = await mountAt("/received/r1");
    await w.find('[data-test="delete"]').trigger("click");
    await flushPromises();
    expect(calls(fetch)).toContain("DELETE /api/documents/r1");
    expect(router.currentRoute.value.fullPath).toBe("/received?type=credit_note");
  });

  it("redirects an issued document to the issued detail", async () => {
    mockFetchRoutes({ "GET /api/documents/d1": document({ status: "issued" }), "GET /api/documents/d1/payments": [] });
    const { router } = await mountAt("/received/d1");
    expect(router.currentRoute.value.fullPath).toBe("/invoices/d1");
  });

  it("labels related received documents as received, in their own currency when given", async () => {
    const ddpp = document({
      id: "r2",
      direction: "received",
      docType: "advance_tax_doc",
      status: "issued",
      number: "PDP20260001",
      dueDate: null,
      parent: { id: "r1", docType: "proforma", number: "PZ20260001", status: "issued", payable: "1434.00", currency: "EUR" },
    });
    mockFetchRoutes({ "GET /api/documents/r2": ddpp, "GET /api/settings/categories": [], "GET /api/settings/custom-fields": [] });
    const { w } = await mountAt("/received/r2");
    const parent = w.find('[data-test="related-parent"]');
    expect(parent.text()).toContain("Received proforma PZ20260001");
    expect(parent.text()).toContain("Recorded");
    expect(parent.text()).toContain("€1,434.00");
    expect(parent.find("a").attributes("href")).toBe("/received/r1");
    expect(w.find('[data-test="payments-panel"]').exists()).toBe(false);
  });
});

