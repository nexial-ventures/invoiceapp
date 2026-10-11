import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia } from "pinia";
import { i18n } from "@/i18n";
import { mockFetchRoutes, reply } from "@/test-utils";
import { company as baseCompany } from "@/features/documents/testData";
import type { Company } from "../types";
import CompanyTab from "./CompanyTab.vue";

const company: Company = {
  ...baseCompany,
  defaultPaymentMethod: "card",
  defaultCurrency: "EUR",
  defaultRoundingCash: "half",
  defaultHeaderNote: { cs: "Děkujeme", en: "Thank you" },
};

async function mountTab() {
  const w = mount(CompanyTab, { global: { plugins: [createPinia(), i18n] } });
  await flushPromises();
  return w;
}
const value = (w: VueWrapper, sel: string) => (w.find(sel).element as HTMLInputElement).value;
const putBody = (fetch: ReturnType<typeof mockFetchRoutes>) =>
  JSON.parse(String(fetch.mock.calls.find(([, init]) => init?.method === "PUT")?.[1]?.body)) as Company;

describe("CompanyTab document defaults", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("shows the stored defaults with the notes per language tab", async () => {
    mockFetchRoutes({ "GET /api/settings/company": company });
    const w = await mountTab();
    const section = w.find('[data-test="document-defaults"]');
    expect(section.find("h2").text()).toBe("Document defaults");
    expect(value(w, "#company-defaultPaymentMethod")).toBe("card");
    expect(value(w, "#company-defaultCurrency")).toBe("EUR");
    expect(value(w, "#company-defaultRoundingCash")).toBe("half");
    expect(value(w, "#company-defaultRoundingOther")).toBe("none");
    expect(value(w, "#company-defaultHeaderNote-cs")).toBe("Děkujeme");
    await w.find('[data-test="notes-tab-en"]').trigger("click");
    expect(value(w, "#company-defaultHeaderNote-en")).toBe("Thank you");
    expect(w.find("#company-defaultHeaderNote-cs").exists()).toBe(false);
  });

  it("saves the defaults with both languages' notes", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/settings/company": company,
      "PUT /api/settings/company": (body: unknown) => body,
    });
    const w = await mountTab();
    await w.find("#company-defaultRoundingOther").setValue("unit");
    await w.find("#company-defaultCurrency").setValue("usd");
    await w.find("#company-defaultFooterNote-cs").setValue("Zápis v OR");
    await w.find('[data-test="notes-tab-en"]').trigger("click");
    await w.find("#company-defaultHeaderNote-en").setValue(" ");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(putBody(fetch)).toMatchObject({
      defaultPaymentMethod: "card",
      defaultCurrency: "USD",
      defaultRoundingCash: "half",
      defaultRoundingOther: "unit",
      defaultHeaderNote: { cs: "Děkujeme", en: null },
      defaultFooterNote: { cs: "Zápis v OR", en: null },
    });
  });

  it("maps a 422 onto the fields, switching to the language tab of a failing note", async () => {
    mockFetchRoutes({
      "GET /api/settings/company": company,
      "PUT /api/settings/company": reply(422, {
        code: "validation",
        fields: { defaultCurrency: "invalid", "defaultFooterNote.en": "too_long" },
      }),
    });
    const w = await mountTab();
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(w.find("#company-defaultCurrency-error").text()).toBe("Invalid value.");
    expect(w.find('[data-test="notes-tab-en"]').attributes("aria-selected")).toBe("true");
    expect(w.find("#company-defaultFooterNote-en-error").exists()).toBe(true);
    expect(w.find("#company-defaultHeaderNote-en-error").exists()).toBe(false);
  });
});
