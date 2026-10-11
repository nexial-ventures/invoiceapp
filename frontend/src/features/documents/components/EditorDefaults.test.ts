import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { i18n } from "@/i18n";
import { mockFetchRoutes } from "@/test-utils";
import type { Company } from "@/features/settings/types";
import { newDocumentDraft } from "../form";
import { newItemLine } from "../lines";
import { company, ctx, totals } from "../testData";
import InvoiceEditorForm from "./InvoiceEditorForm.vue";

const withDefaults: Company = {
  ...company,
  defaultRoundingCash: "unit",
  defaultRoundingOther: "none",
  defaultHeaderNote: { cs: "Děkujeme", en: "Thank you" },
  defaultFooterNote: { cs: null, en: "Registered in CR" },
};

function mountNew() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/:p(.*)*", name: "contact-new", component: { template: "<div />" } }] });
  const c = ctx({ company: withDefaults });
  const initial = { ...newDocumentDraft(c), lines: [{ ...newItemLine("21"), description: "A" }] };
  return mount(InvoiceEditorForm, { props: { initial, ctx: c, categories: [] }, global: { plugins: [pinia, i18n, router] } });
}

const value = (w: VueWrapper, sel: string) => (w.find(sel).element as HTMLInputElement).value;
const computeBodies = (fetch: ReturnType<typeof mockFetchRoutes>) =>
  fetch.mock.calls.filter(([, init]) => init?.method === "POST").map(([, init]) => JSON.parse(String(init?.body)) as { roundingMode: string });

describe("editor rounding and defaults", () => {
  let fetch: ReturnType<typeof mockFetchRoutes>;
  beforeEach(() => {
    i18n.global.locale.value = "cs";
    vi.useFakeTimers();
    fetch = mockFetchRoutes({ "POST /api/documents/compute": { lines: [], totals: totals() } });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("offers the three modes, for any currency, and computes with the picked one", async () => {
    const w = mountNew();
    const options = w.findAll("#doc-roundingMode option").map((o) => o.text());
    expect(options).toEqual(["Bez zaokrouhlení", "Na 0,50", "Na celé jednotky"]);
    await w.find("#doc-currency").setValue("EUR");
    await w.find("#doc-currency").trigger("change");
    expect(value(w, "#doc-currency")).toBe("EUR");
    expect(w.find("#doc-roundingMode").attributes("disabled")).toBeUndefined();
    await w.find("#doc-roundingMode").setValue("half");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(computeBodies(fetch).at(-1)?.roundingMode).toBe("half");
  });

  it("payment method re-applies the company rounding until the select is changed by hand", async () => {
    const w = mountNew();
    expect(value(w, "#doc-roundingMode")).toBe("none");
    await w.find("#doc-paymentMethod").setValue("cash");
    expect(value(w, "#doc-roundingMode")).toBe("unit");
    await w.find("#doc-paymentMethod").setValue("card");
    expect(value(w, "#doc-roundingMode")).toBe("none");

    await w.find("#doc-roundingMode").setValue("half");
    await w.find("#doc-paymentMethod").setValue("cash");
    expect(value(w, "#doc-roundingMode")).toBe("half");
  });

  it("locale swaps the default notes unless edited", async () => {
    const w = mountNew();
    expect(value(w, "#doc-headerNote")).toBe("Děkujeme");
    await w.find("#doc-locale").setValue("en");
    expect([value(w, "#doc-headerNote"), value(w, "#doc-footerNote")]).toEqual(["Thank you", "Registered in CR"]);

    await w.find("#doc-footerNote").setValue("Own text");
    await w.find("#doc-locale").setValue("cs");
    expect([value(w, "#doc-headerNote"), value(w, "#doc-footerNote")]).toEqual(["Děkujeme", "Own text"]);
  });

  it("a cleared default note is saved as no note on create", async () => {
    fetch = mockFetchRoutes({
      "POST /api/documents/compute": { lines: [], totals: totals() },
      "POST /api/documents": { id: "new" },
    });
    const w = mountNew();
    await w.find("#doc-headerNote").setValue("");
    await w.find("form").trigger("submit");
    await flushPromises();
    const post = fetch.mock.calls.find(([url, init]) => init?.method === "POST" && String(url).endsWith("/api/documents"));
    expect(JSON.parse(String(post?.[1]?.body))).toMatchObject({ headerNote: "", footerNote: "" });
  });
});
