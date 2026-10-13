import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { i18n } from "@/i18n";
import { mockFetchRoutes, reply } from "@/test-utils";
import { bankAccounts, company, document, totals, vatRates } from "../testData";
import InvoiceEditView from "./InvoiceEditView.vue";

async function mountAt(path: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const stub = { template: "<div />" };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/invoices", name: "invoices", component: stub },
      { path: "/invoices/new", name: "invoice-new", component: InvoiceEditView },
      { path: "/invoices/:id", name: "invoice-detail", component: stub },
      { path: "/invoices/:id/edit", name: "invoice-edit", component: InvoiceEditView },
      { path: "/contacts/new", name: "contact-new", component: stub },
    ],
  });
  await router.push(path);
  const w = mount({ template: "<RouterView />" }, { global: { plugins: [pinia, i18n, router] } });
  await flushPromises();
  return { w, router };
}

describe("InvoiceEditView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    mockFetchRoutes({
      "GET /api/settings/company": company,
      "GET /api/settings/vat-rates": vatRates,
      "GET /api/settings/bank-accounts": bankAccounts,
      "GET /api/settings/categories": [],
      "GET /api/settings/custom-fields": [],
      "POST /api/documents/compute": { lines: [], totals: totals() },
    });
  });
  afterEach(() => vi.unstubAllGlobals());

  it("new invoice by default", async () => {
    const { w } = await mountAt("/invoices/new");
    expect(w.find("h1").text()).toBe("New invoice");
    expect(w.find("#doc-taxPointDate").exists()).toBe(true);
  });

  it("?docType=proforma opens the proforma mode without a tax point date", async () => {
    const { w } = await mountAt("/invoices/new?docType=proforma");
    expect(w.find("h1").text()).toBe("New proforma invoice");
    expect(w.find("#doc-taxPointDate").exists()).toBe(false);
    expect(w.find("a").attributes("href")).toBe("/invoices?type=proforma");
  });

  it("import mode: any type, own required number, optional link, imported flag on save", async () => {
    const invoices = { items: [{ ...document({ id: "i1", number: "2026-0042", status: "issued" }), customerName: "Acme" }], total: 1 };
    let answer: unknown = reply(422, { code: "validation", fields: { number: "duplicate" } });
    const fetch = mockFetchRoutes({
      "GET /api/settings/company": company,
      "GET /api/settings/vat-rates": vatRates,
      "GET /api/settings/bank-accounts": bankAccounts,
      "GET /api/settings/categories": [],
      "GET /api/settings/custom-fields": [],
      "GET /api/documents": invoices,
      "POST /api/documents/compute": { lines: [], totals: totals() },
      "POST /api/documents": () => answer,
    });
    const { w, router } = await mountAt("/invoices/new?docType=credit_note&imported=1");
    expect(w.find("h1").text()).toBe("Import a credit note");
    expect(w.find('[data-test="import-section"]').exists()).toBe(true);
    // An imported credit note is entered as printed: currency and customer are not locked.
    expect(w.find("#doc-currency").attributes("disabled")).toBeUndefined();
    expect(w.find("#contact-search").exists()).toBe(true);
    // Only issued (non-draft) invoices can be linked.
    expect(fetch.mock.calls.some(([url]) => String(url) === "/api/documents?direction=issued&docType=invoice&status=issued&limit=50&offset=0")).toBe(true);

    await w.find("#doc-correctionReason").setValue("Discount");
    await w.find('[data-test="save-document"]').trigger("submit");
    expect(w.find("#doc-number-error").text()).toBe("This field is required.");

    await w.find("#doc-number").setValue(" DOB-7 ");
    await w.find('[data-test="related-picker"]').setValue("i1");
    await w.find("form").trigger("submit");
    await flushPromises();
    const post = fetch.mock.calls.find(([url, init]) => url === "/api/documents" && init?.method === "POST");
    expect(JSON.parse(String(post?.[1]?.body))).toMatchObject({ docType: "credit_note", imported: true, number: "DOB-7", relatedDocumentId: "i1" });
    expect(w.find("#doc-number-error").text()).toBe("This value already exists.");

    answer = reply(201, document({ id: "n1", imported: true }));
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(router.currentRoute.value.fullPath).toBe("/invoices/n1");
  });

  it("native drafts send imported false, no number and no relatedDocumentId", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/settings/company": company,
      "GET /api/settings/vat-rates": vatRates,
      "GET /api/settings/bank-accounts": bankAccounts,
      "GET /api/settings/categories": [],
      "GET /api/settings/custom-fields": [],
      "POST /api/documents/compute": { lines: [], totals: totals() },
      "POST /api/documents": reply(201, document({ id: "n2" })),
    });
    const { w } = await mountAt("/invoices/new");
    expect(w.find('[data-test="import-section"]').exists()).toBe(false);
    await w.find("form").trigger("submit");
    await flushPromises();
    const body = JSON.parse(String(fetch.mock.calls.find(([url, init]) => url === "/api/documents" && init?.method === "POST")?.[1]?.body)) as Record<string, unknown>;
    expect(body).toMatchObject({ imported: false, number: null, categoryId: null, customFields: {} });
    expect("relatedDocumentId" in body).toBe(false);
  });

  it("an unlocked draft shows the unlock and the number it keeps", async () => {
    const draft = document({
      number: "20260001",
      unlockedAt: "2026-10-12T09:30:00Z",
      unlockedBy: { email: "admin@example.com", displayName: "" },
      unlockReason: "Wrong VAT rate",
    });
    mockFetchRoutes({
      "GET /api/settings/company": company,
      "GET /api/settings/vat-rates": vatRates,
      "GET /api/settings/bank-accounts": bankAccounts,
      "GET /api/settings/categories": [],
      "GET /api/settings/custom-fields": [],
      "GET /api/documents/d1": draft,
      "GET /api/contacts/c1": { id: "c1", name: "Acme" },
      "POST /api/documents/compute": { lines: [], totals: totals() },
    });
    const { w } = await mountAt("/invoices/d1/edit");
    const banner = w.find('[data-test="unlocked-banner"]').text();
    expect(banner).toContain("admin@example.com");
    expect(banner).toContain("Wrong VAT rate");
    expect(w.find('[data-test="kept-number"]').text()).toContain("20260001");
  });
});
