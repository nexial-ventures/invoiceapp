import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { i18n } from "@/i18n";
import { mockFetchRoutes, reply } from "@/test-utils";
import { changeCurrency, newDocumentDraft, type DocumentDraft } from "../form";
import { newItemLine } from "../lines";
import { bankAccounts, ctx, totals } from "../testData";
import InvoiceEditorForm from "./InvoiceEditorForm.vue";

function mountForm(transform: (d: DocumentDraft) => DocumentDraft = (d) => d) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/:p(.*)*", name: "contact-new", component: { template: "<div />" } }] });
  const initial = {
    ...newDocumentDraft(ctx()),
    lines: [
      { ...newItemLine("21"), description: "A" },
      { ...newItemLine("21"), description: "B" },
    ],
  };
  return mount(InvoiceEditorForm, { props: { initial: transform(initial), ctx: ctx(), categories: [], docId: "d1" }, global: { plugins: [pinia, i18n, router] } });
}

describe("InvoiceEditorForm", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("shows live totals from compute", async () => {
    const fetch = mockFetchRoutes({
      "POST /api/documents/compute": { lines: [], totals: totals("1210.00") },
      "GET /api/documents/number-preview": { number: "20260001", source: "global" },
    });
    const w = mountForm();
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    const computes = fetch.mock.calls.filter(([url]) => url === "/api/documents/compute");
    expect(computes).toHaveLength(1);
    expect(JSON.parse(String(computes[0]?.[1]?.body))).toMatchObject({ documentId: "d1", docType: "invoice", locale: "cs", contactId: null });
    expect(w.find('[data-test="payable"]').text().replace(/\s/g, " ")).toBe("CZK 1,210.00");
  });

  it("maps a save 422 onto header and the right line inputs", async () => {
    const fetch = mockFetchRoutes({
      "POST /api/documents/compute": { lines: [], totals: totals() },
      "PUT /api/documents/d1": reply(422, {
        code: "validation",
        fields: { contactId: "required", "lines.1.quantity": "invalid", "lines.0.vatRate": "invalid" },
      }),
    });
    const w = mountForm();
    await w.find("form").trigger("submit");
    await flushPromises();

    const put = fetch.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toMatchObject({ docType: "invoice", lines: [{ description: "A" }, { description: "B" }] });
    expect(w.find("#contact-search-error").text()).toBe("This field is required.");
    expect(w.find("#line-1-quantity-error").text()).toBe("Invalid value.");
    expect(w.find("#line-0-vatRate-error").text()).toBe("Invalid value.");
    expect(w.find("#line-0-quantity-error").exists()).toBe(false);
    expect(w.emitted("saved")).toBeUndefined();

    // Reordering invalidates index-based server errors.
    await w.find('[data-test="line-0"] [data-test="move-down"]').trigger("click");
    expect(w.find("#line-0-quantity-error").exists()).toBe(false);
    expect(w.find("#line-1-quantity-error").exists()).toBe(false);
  });

  it("shows compute 422 errors on the matching line inputs", async () => {
    const fetch = mockFetchRoutes({
      "POST /api/documents/compute": reply(422, {
        code: "validation",
        fields: { "lines.0.unitPrice": "invalid", "lines.1.vatRate": "invalid" },
      }),
    });
    const w = mountForm();
    await w.find("#line-1-description").setValue("");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    const sent = JSON.parse(String(fetch.mock.calls.at(-1)?.[1]?.body)) as { lines: { description: string }[] };
    expect(sent.lines.map((l) => l.description)).toEqual(["A", ""]);
    expect(w.find("#line-0-unitPrice-error").text()).toBe("Invalid value.");
    expect(w.find("#line-1-vatRate-error").text()).toBe("Invalid value.");
    expect(w.find('[data-test="totals-panel"] [role="alert"]').text()).toBe("Fix the highlighted fields to recalculate the totals.");
  });

  it("previews CZK with the indicative ČNB rate but never saves it", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/exchange-rates/EUR": { currency: "EUR", date: "2026-10-07", rate: "24.4" },
      "POST /api/documents/compute": { lines: [], totals: totals() },
      "PUT /api/documents/d1": { id: "d1" },
    });
    const w = mountForm((d) => changeCurrency(d, "EUR", bankAccounts));
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(w.find('[data-test="indicative-rate"]').text()).toBe("Indicative ČNB rate for 7 Oct 2026: 24.4 (fixed at issue)");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    const bodies = (method: string) =>
      fetch.mock.calls.filter(([, init]) => init?.method === method).map(([, init]) => JSON.parse(String(init?.body)) as { exchangeRate: string | null });
    expect(bodies("POST").at(-1)?.exchangeRate).toBe("24.4");

    await w.find("form").trigger("submit");
    await flushPromises();
    expect(bodies("PUT")[0]?.exchangeRate).toBeNull();
    expect((w.find("#doc-exchangeRate").element as HTMLInputElement).value).toBe("");
  });

  it("shows an inline note when ČNB is unavailable and stays usable", async () => {
    mockFetchRoutes({
      "GET /api/exchange-rates/EUR": reply(502, { code: "cnb_unavailable" }),
      "POST /api/documents/compute": { lines: [], totals: totals() },
    });
    const w = mountForm((d) => changeCurrency(d, "EUR", bankAccounts));
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(w.find('[data-test="indicative-rate"]').text()).toContain("cannot be loaded");
    expect(w.find("#doc-exchangeRate").attributes("disabled")).toBeUndefined();
  });

  it("proforma: no tax point field, saved with docType proforma and no tax point", async () => {
    const fetch = mockFetchRoutes({ "POST /api/documents/compute": { lines: [], totals: totals() }, "PUT /api/documents/d1": { id: "d1" } });
    const w = mountForm((d) => ({ ...d, docType: "proforma", taxPointDate: "" }));
    expect(w.find("#doc-taxPointDate").exists()).toBe(false);
    await w.find("form").trigger("submit");
    await flushPromises();
    const put = fetch.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toMatchObject({ docType: "proforma", taxPointDate: null });
  });

  it("credit note: reason required, currency and rate fixed, totals negated", async () => {
    const fetch = mockFetchRoutes({ "POST /api/documents/compute": { lines: [], totals: totals() }, "PUT /api/documents/d1": { id: "d1" } });
    const w = mountForm((d) => ({ ...changeCurrency(d, "EUR", bankAccounts), docType: "credit_note", exchangeRate: "24.3" }));
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(fetch.mock.calls.some(([url]) => String(url).startsWith("/api/exchange-rates"))).toBe(false);
    expect(w.find("#doc-exchangeRate").exists()).toBe(false);
    expect(w.find('[data-test="original-rate"]').text()).toContain("24.3");
    expect(w.find("#doc-currency").attributes("disabled")).toBeDefined();
    expect(w.find("#doc-vatMode").attributes("disabled")).toBeDefined();
    expect(w.find('[data-test="payable"]').text()).toMatch(/^-/);

    await w.find("form").trigger("submit");
    await flushPromises();
    expect(w.find("#doc-correctionReason-error").text()).toBe("This field is required.");
    expect(fetch.mock.calls.some(([, init]) => init?.method === "PUT")).toBe(false);

    await w.find("#doc-correctionReason").setValue("wrong price");
    await w.find("form").trigger("submit");
    await flushPromises();
    const put = fetch.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toMatchObject({ docType: "credit_note", correctionReason: "wrong price", exchangeRate: "24.3" });
  });

  it("debit note: reason with its hint, customer locked, totals positive", async () => {
    mockFetchRoutes({ "POST /api/documents/compute": { lines: [], totals: totals() }, "GET /api/contacts/c1": { id: "c1", name: "Acme" } });
    const w = mountForm((d) => ({ ...d, docType: "debit_note", contactId: "c1" }));
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(w.find("#doc-correctionReason").exists()).toBe(true);
    expect(w.text()).toContain("Enter the items the original document is increased by.");
    expect(w.find("#doc-currency").attributes("disabled")).toBeDefined();
    expect(w.text()).not.toContain("Change");
    expect(w.find('[data-test="payable"]').text()).not.toMatch(/^-/);
  });

  it("simplified: customer optional, warning above 10 000 CZK, saves without a contact", async () => {
    const fetch = mockFetchRoutes({ "POST /api/documents/compute": { lines: [], totals: totals("12100.00") }, "PUT /api/documents/d1": { id: "d1" } });
    const w = mountForm((d) => ({ ...d, docType: "simplified" }));
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(w.find("#contact-picker-label").text()).toBe("Customer (optional)");
    expect(w.find('[data-test="simplified-limit"]').exists()).toBe(true);

    await w.find("form").trigger("submit");
    await flushPromises();
    const put = fetch.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toMatchObject({ docType: "simplified", contactId: null, correctionReason: null });
  });
});
