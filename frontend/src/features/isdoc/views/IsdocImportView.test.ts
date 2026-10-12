import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { i18n } from "@/i18n";
import { mockFetchRoutes, reply } from "@/test-utils";
import { entry, isdocFile } from "../testData";
import { MAX_UPLOAD_BYTES } from "@/features/imports/upload";
import IsdocImportView from "./IsdocImportView.vue";

const categories = [
  { id: "cat1", name: "Office", kind: "expense", active: true, position: 0 },
  { id: "cat2", name: "Sales", kind: "income", active: true, position: 1 },
  { id: "cat3", name: "Old", kind: "expense", active: false, position: 2 },
];

const preview = {
  entries: [
    entry({ key: "a.zip/1.isdoc", number: "20260001" }),
    entry({ key: "a.zip/2.isdoc", direction: "received", number: "FV-7", contactMatch: "new", warnings: ["contact_created", "related_not_found"], relatedNumber: "FV-1" }),
    entry({ key: "a.zip/3.isdoc", status: "duplicate" }),
    entry({ key: "a.zip/4.isdoc", status: "error", error: "foreign", direction: null, docType: null, number: null, counterparty: null, contactMatch: null, total: null }),
  ],
};

async function mountView(path = "/import/isdoc") {
  setActivePinia(createPinia());
  const stub = { template: "<div />" };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/import/isdoc", name: "isdoc-import", component: IsdocImportView },
      { path: "/invoices", name: "invoices", component: stub },
      { path: "/invoices/:id", name: "invoice-detail", component: stub },
      { path: "/received", name: "received", component: stub },
      { path: "/received/:id", name: "received-detail", component: stub },
    ],
  });
  await router.push(path);
  const w = mount({ template: "<RouterView />" }, { global: { plugins: [i18n, router] } });
  await flushPromises();
  return w;
}

async function pick(w: VueWrapper, files: File[]) {
  const input = w.find('[data-test="import-input"]');
  Object.defineProperty(input.element, "files", { value: files, configurable: true });
  await input.trigger("change");
  await flushPromises();
}

const rows = (w: VueWrapper) => w.findAll('[data-test="isdoc-preview-row"]');
const checkbox = (w: VueWrapper, i: number) => rows(w)[i]!.find<HTMLInputElement>('[data-test="isdoc-select"]');

describe("IsdocImportView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("previews the picked files: ok rows preselected, others disabled, codes translated", async () => {
    const fetch = mockFetchRoutes({ "POST /api/import/isdoc/preview": preview, "GET /api/settings/categories": categories });
    const w = await mountView();
    await pick(w, [isdocFile("a.zip"), isdocFile("notes.txt")]);

    expect((fetch.mock.calls[0]![1]!.body as FormData).getAll("files")).toHaveLength(1);
    expect(w.find('[data-test="isdoc-ignored"]').text()).toContain("notes.txt");
    expect(rows(w)).toHaveLength(4);
    expect([0, 1, 2, 3].map((i) => checkbox(w, i).element.checked)).toEqual([true, true, false, false]);
    expect([0, 1, 2, 3].map((i) => checkbox(w, i).element.disabled)).toEqual([false, false, true, true]);
    expect(rows(w)[1]!.find('[data-test="import-contact"]').text()).toBe("New contact");
    expect(rows(w)[1]!.find('[data-test="isdoc-warnings"]').text()).toContain("Original document FV-1 was not found");
    expect(rows(w)[1]!.text()).toContain("Received invoice");
    expect(rows(w)[3]!.find('[data-test="isdoc-error"]').text()).toBe("Your company's IČO is neither the supplier's nor the customer's.");
    expect(w.find('[data-test="import-confirm"]').text()).toBe("Import selected (2)");
  });

  it("shows the received options only while a received row is selected; expense categories only", async () => {
    mockFetchRoutes({ "POST /api/import/isdoc/preview": preview, "GET /api/settings/categories": categories });
    const w = await mountView();
    await pick(w, [isdocFile("a.zip")]);

    expect(w.find<HTMLInputElement>('[data-test="isdoc-mark-paid"]').element.checked).toBe(true);
    expect(w.find<HTMLInputElement>('[data-test="isdoc-vat-deductible"]').element.checked).toBe(true);
    expect(w.findAll('[data-test="isdoc-category"] option').map((o) => o.text())).toEqual(["No category", "Office"]);

    await checkbox(w, 1).setValue(false);
    expect(w.find('[data-test="isdoc-received-options"]').exists()).toBe(false);
    await w.find('[data-test="isdoc-select-all"]').setValue(true);
    expect(w.find('[data-test="isdoc-received-options"]').exists()).toBe(true);
    await w.find('[data-test="isdoc-select-all"]').setValue(false);
    expect(w.find<HTMLButtonElement>('[data-test="import-confirm"]').element.disabled).toBe(true);
  });

  it("confirms with the same files + options and links the imported documents", async () => {
    const fetch = mockFetchRoutes({
      "POST /api/import/isdoc/preview": preview,
      "GET /api/settings/categories": categories,
      "POST /api/import/isdoc/confirm": reply(200, {
        results: [
          { key: "a.zip/1.isdoc", status: "imported", documentId: "d1", error: null },
          { key: "a.zip/2.isdoc", status: "imported", documentId: "d2", error: null },
          { key: "a.zip/2.isdoc#2", status: "failed", documentId: null, error: "duplicate" },
          { key: "a.zip/3.isdoc", status: "skipped", documentId: null, error: null },
          { key: "a.zip/4.isdoc", status: "failed", documentId: null, error: "rate_unavailable" },
        ],
      }),
    });
    const w = await mountView();
    const file = isdocFile("a.zip");
    await pick(w, [file]);
    await w.find('[data-test="isdoc-mark-paid"]').setValue(false);
    await w.find('[data-test="isdoc-category"]').setValue("cat1");
    await w.find('[data-test="isdoc-vat-deductible"]').setValue(false);
    await w.find('[data-test="import-confirm"]').trigger("click");
    await flushPromises();

    const body = fetch.mock.calls.find(([url]) => url === "/api/import/isdoc/confirm")![1]!.body as FormData;
    expect(body.getAll("files")).toEqual([file]);
    expect(JSON.parse(body.get("options") as string)).toEqual({ selected: ["a.zip/1.isdoc", "a.zip/2.isdoc"], markPaid: false, categoryId: "cat1", vatDeductible: false });

    expect(w.find('[data-test="import-summary"]').text()).toBe("Imported 2 of 5.");
    const links = w.findAll('[data-test="import-result-link"]').map((l) => l.attributes("href"));
    expect(links).toEqual(["/invoices/d1", "/received/d2"]);
    expect(w.findAll('[data-test="import-result-error"]').map((e) => e.text())).toEqual([
      "The document already exists (created meanwhile).",
      "The ČNB exchange rates are unavailable. Try the import later.",
    ]);
    expect(w.find('[data-test="isdoc-preview"]').exists()).toBe(false);
  });

  it("issued-only selection sends no category and default VAT deductible", async () => {
    const fetch = mockFetchRoutes({
      "POST /api/import/isdoc/preview": preview,
      "GET /api/settings/categories": categories,
      "POST /api/import/isdoc/confirm": { results: [] },
    });
    const w = await mountView();
    await pick(w, [isdocFile("a.zip")]);
    await w.find('[data-test="isdoc-category"]').setValue("cat1");
    await checkbox(w, 1).setValue(false);
    await w.find('[data-test="import-confirm"]').trigger("click");
    await flushPromises();
    const body = fetch.mock.calls.find(([url]) => url === "/api/import/isdoc/confirm")![1]!.body as FormData;
    expect(JSON.parse(body.get("options") as string)).toEqual({ selected: ["a.zip/1.isdoc"], markPaid: true, categoryId: null, vatDeductible: true });
  });

  it("explains the upload limits; nothing is sent for unsupported or oversized picks", async () => {
    let answer = reply(413, { code: "too_large" });
    const fetch = mockFetchRoutes({ "POST /api/import/isdoc/preview": () => answer, "GET /api/settings/categories": categories });
    const w = await mountView("/import/isdoc?from=received");
    expect(w.find('[data-test="import-back"]').attributes("href")).toBe("/received");
    const alert = () => w.find('[data-test="import-error-alert"]').text();

    await pick(w, [isdocFile("a.pdf")]);
    expect(alert()).toBe("None of the chosen files is an .isdoc, .isdocx or .zip.");
    await pick(w, [isdocFile("big.zip", MAX_UPLOAD_BYTES + 1)]);
    expect(alert()).toBe("The files are too large together (50 MB at most).");
    expect(fetch.mock.calls.filter(([url]) => String(url).startsWith("/api/import"))).toHaveLength(0);

    await pick(w, [isdocFile("a.zip")]);
    expect(alert()).toBe("The files are too large together (50 MB at most).");
    answer = reply(422, { code: "validation", fields: { files: "too_many" } });
    await pick(w, [isdocFile("a.zip")]);
    expect(alert()).toContain("500 at most");
    answer = reply(422, { code: "validation", fields: { files: "too_large" } });
    await pick(w, [isdocFile("a.zip")]);
    expect(alert()).toContain("200 MB at most");
    answer = reply(422, { code: "quota_exceeded", detail: "99.0 / 100 MB" });
    await pick(w, [isdocFile("a.zip")]);
    expect(alert()).toContain("storage quota of the space (99.0 / 100 MB)");
  });
});
