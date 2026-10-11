import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { i18n } from "@/i18n";
import { contact, document, vatRates } from "@/features/documents/testData";
import type { CustomField } from "@/features/settings/types";
import { mockFetchRoutes, reply } from "@/test-utils";
import ReceivedEditView from "./ReceivedEditView.vue";

const poField: CustomField = { id: "f1", key: "po", label: "PO", type: "text", options: [], appliesTo: "received", required: false, active: true, position: 0 };

async function mountAt(path: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const stub = { template: "<div />" };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/received", name: "received", component: stub },
      { path: "/received/new", name: "received-new", component: ReceivedEditView },
      { path: "/received/:id", name: "received-detail", component: stub },
      { path: "/received/:id/edit", name: "received-edit", component: ReceivedEditView },
      { path: "/invoices/:id", name: "invoice-detail", component: stub },
      { path: "/contacts/new", name: "contact-new", component: stub },
    ],
  });
  await router.push(path);
  const w = mount({ template: "<RouterView />" }, { global: { plugins: [pinia, i18n, router] } });
  await flushPromises();
  return { w, router };
}

const routes = (extra: Record<string, unknown> = {}) =>
  mockFetchRoutes({
    "GET /api/settings/vat-rates": vatRates,
    "GET /api/settings/categories": [{ id: "k1", name: "Software", kind: "expense", active: true, position: 0 }, { id: "k2", name: "Sales", kind: "income", active: true, position: 0 }],
    "GET /api/settings/custom-fields": [poField],
    "GET /api/contacts": { items: [contact()], total: 1 },
    "GET /api/documents": { items: [], total: 0 },
    ...extra,
  });

describe("ReceivedEditView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("shows the date fields per doc type", async () => {
    routes();
    let { w } = await mountAt("/received/new?docType=proforma");
    expect(w.find("h1").text()).toBe("Record a proforma");
    expect(w.find("#rec-taxPointDate").exists()).toBe(false);
    expect(w.find("#rec-dueDate").exists()).toBe(true);
    ({ w } = await mountAt("/received/new?docType=advance_tax_doc"));
    expect(w.find("#rec-taxPointDate").exists()).toBe(true);
    expect(w.find("#rec-dueDate").exists()).toBe(false);
  });

  it("follows the tax point with the received date and the total with the payable until edited", async () => {
    routes();
    const { w } = await mountAt("/received/new");
    await w.find("#rec-taxPointDate").setValue("2026-09-30");
    expect((w.find("#rec-receivedDate").element as HTMLInputElement).value).toBe("2026-09-30");
    await w.find("#rec-receivedDate").setValue("2026-10-02");
    await w.find("#rec-taxPointDate").setValue("2026-09-29");
    expect((w.find("#rec-receivedDate").element as HTMLInputElement).value).toBe("2026-10-02");

    await w.find("#recap-0-base").setValue("1000");
    expect((w.find("#recap-0-vat").element as HTMLInputElement).value).toBe("210.00");
    expect(w.find('[data-test="recap-total"]').text()).toContain("1,210.00");
    expect((w.find("#rec-payable").element as HTMLInputElement).value).toBe("1210.00");
    await w.find("#rec-payable").setValue("500");
    await w.find("#recap-0-base").setValue("2000");
    expect((w.find("#rec-payable").element as HTMLInputElement).value).toBe("500");

    // The mode is read-only, derived from the rounding amount as the server does on save.
    const roundingHint = () => w.find("#rec-rounding").element.parentElement?.textContent ?? "";
    expect(roundingHint()).toContain("Mode: No rounding");
    await w.find("#recap-0-base").setValue("1000.4");
    await w.find("#rec-rounding").setValue("0.02");
    expect(roundingHint()).toContain("Mode: To 0.50");
    await w.find("#rec-rounding").setValue("-0.48");
    expect(roundingHint()).toContain("Mode: To whole units");

    await w.find("#rec-vatMode").setValue("reverse_charge");
    expect((w.find("#recap-0-vat").element as HTMLInputElement).value).toBe("0");
    expect((w.find("#rec-vatDeductible").element as HTMLInputElement).checked).toBe(false);
  });

  it("saves the received document and routes 422 recap / custom field errors inline", async () => {
    let answer: unknown = reply(422, { code: "validation", fields: { "vatRecap.0.base": "invalid", "customFields.po": "unknown" } });
    const fetch = routes({ "POST /api/documents": () => answer });
    const { w, router } = await mountAt("/received/new");
    await w.find("#contact-search").trigger("focus");
    await flushPromises();
    await w.find('[data-test="contact-results"] button').trigger("click");
    await w.find("#rec-supplierNumber").setValue("FV-1");
    await w.find("#rec-dueDate").setValue("2026-10-30");
    await w.find("#recap-0-base").setValue("100");
    await w.find("#rec-categoryId").setValue("k1");
    await w.find("#rec-cf-po").setValue("PO-7");
    await w.find('[data-test="received-form"]').trigger("submit");
    await flushPromises();

    const body = JSON.parse(String(fetch.mock.calls.find(([, init]) => init?.method === "POST")?.[1]?.body));
    expect(body).toMatchObject({
      direction: "received",
      docType: "invoice",
      contactId: "c1",
      supplierNumber: "FV-1",
      vatRecap: [{ rate: "21", base: "100", vat: "21.00" }],
      payable: "121.00",
      categoryId: "k1",
      customFields: { po: "PO-7" },
    });
    // Only expense categories are offered on a received document.
    expect(w.findAll("#rec-categoryId option").map((o) => o.text())).toEqual(["No category", "Software"]);
    const errors = w.findAll('[data-test="field-error"]').map((e) => e.text());
    expect(errors).toEqual(["Invalid value.", "Unknown field."]);

    answer = reply(201, document({ id: "r9", direction: "received" }));
    await w.find('[data-test="received-form"]').trigger("submit");
    await flushPromises();
    expect(router.currentRoute.value.fullPath).toBe("/received/r9");
  });

  it("edits a stored document from its recap", async () => {
    const stored = document({
      id: "r1",
      direction: "received",
      status: "issued",
      number: "P20260001",
      supplierNumber: "FV-1",
      receivedDate: "2026-10-01",
      vatRecap: [{ rate: "12", base: "100.00", vat: "12.00" }],
      rounding: "0.00",
      payable: "112.00",
    });
    const fetch = routes({ "GET /api/documents/r1": stored, "GET /api/documents/r1/payments": [], "GET /api/contacts/c1": contact(), "PUT /api/documents/r1": stored });
    const { w } = await mountAt("/received/r1/edit");
    expect(w.find("h1").text()).toBe("Edit received invoice");
    expect((w.find("#rec-supplierNumber").element as HTMLInputElement).value).toBe("FV-1");
    expect((w.find("#recap-0-rate").element as HTMLInputElement).value).toBe("12");
    expect((w.find("#rec-payable").element as HTMLInputElement).value).toBe("112.00");

    // ISDOC-imported lines are kept by the server: a received PUT never carries them.
    await w.find('[data-test="received-form"]').trigger("submit");
    await flushPromises();
    const put = fetch.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).not.toHaveProperty("lines");
  });

  it("a filled-in manual rate skips the ČNB lookup and its note; clearing it looks the rate up", async () => {
    const stored = document({
      id: "r1",
      direction: "received",
      status: "issued",
      number: "P20260001",
      supplierNumber: "FV-1",
      currency: "EUR",
      exchangeRate: "25.1",
      exchangeRateSource: "manual",
      receivedDate: "2026-10-01",
      vatRecap: [{ rate: "21", base: "100.00", vat: "21.00" }],
    });
    const fetch = routes({
      "GET /api/documents/r1": stored,
      "GET /api/documents/r1/payments": [],
      "GET /api/contacts/c1": contact(),
      "GET /api/exchange-rates/EUR": reply(502, { code: "cnb_unavailable" }),
    });
    const { w } = await mountAt("/received/r1/edit");
    await new Promise((r) => setTimeout(r, 350));
    await flushPromises();
    const rateCalls = () => fetch.mock.calls.filter(([url]) => String(url).startsWith("/api/exchange-rates"));
    expect((w.find("#rec-exchangeRate").element as HTMLInputElement).value).toBe("25.1");
    expect(rateCalls()).toHaveLength(0);
    expect(w.find('[data-test="indicative-rate"]').exists()).toBe(false);

    await w.find("#rec-exchangeRate").setValue("");
    await new Promise((r) => setTimeout(r, 350));
    await flushPromises();
    expect(rateCalls()).toHaveLength(1);
    expect(w.find('[data-test="indicative-rate"]').exists()).toBe(true);
  });
});
