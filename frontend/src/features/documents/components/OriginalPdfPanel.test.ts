import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import { calls, mockFetchRoutes, reply, signIn } from "@/test-utils";
import { MAX_ORIGINAL_BYTES } from "../original";
import { useDocumentStore } from "../store";
import { document } from "../testData";
import type { Document } from "../types";
import OriginalPdfPanel from "./OriginalPdfPanel.vue";

const received = (overrides: Partial<Document> = {}) =>
  document({ direction: "received", status: "issued", number: "P20260001", ...overrides });

function mountPanel(doc: Document) {
  const pinia = createPinia();
  setActivePinia(pinia);
  signIn("owner");
  const store = useDocumentStore();
  store.doc = doc;
  const w = mount({ components: { OriginalPdfPanel }, template: '<OriginalPdfPanel v-if="store.doc" :doc="store.doc" />', setup: () => ({ store }) }, { global: { plugins: [pinia, i18n] } });
  return { w, store };
}

async function pick(w: ReturnType<typeof mountPanel>["w"], file: File) {
  const input = w.find('[data-test="original-input"]');
  Object.defineProperty(input.element, "files", { value: [file], configurable: true });
  await input.trigger("change");
  await flushPromises();
}

const pdfFile = (size = 10, name = "doc.pdf", type = "application/pdf") => {
  const file = new File(["%PDF-1.7"], name, { type });
  Object.defineProperty(file, "size", { value: size });
  return file;
};

describe("OriginalPdfPanel", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("without an original shows the missing hint and only the upload button", () => {
    mockFetchRoutes({});
    const { w } = mountPanel(received());
    expect(w.find('[data-test="original-missing"]').text()).toBe("No PDF uploaded.");
    expect(w.find('[data-test="original-open"]').exists()).toBe(false);
    expect(w.find('[data-test="original-upload"]').text()).toBe("Upload PDF");
  });

  it("uploads the picked PDF as multipart and reloads the document", async () => {
    const uploaded = received({ original: { sha256: "ab", size: 2048, uploadedAt: "2026-10-08T10:00:00Z" } });
    const fetch = mockFetchRoutes({
      "PUT /api/documents/d1/original": reply(200, {}),
      "GET /api/documents/d1": uploaded,
      "GET /api/documents/d1/payments": [],
    });
    const { w } = mountPanel(received());
    await pick(w, pdfFile());

    const put = fetch.mock.calls.find(([, init]) => init?.method === "PUT");
    const body = put?.[1]?.body as FormData;
    expect(body).toBeInstanceOf(FormData);
    expect((body.get("file") as File).name).toBe("doc.pdf");
    expect(new Headers(put?.[1]?.headers).has("Content-Type")).toBe(false);
    expect(calls(fetch)).toContain("GET /api/documents/d1");
    expect(w.find('[data-test="original-info"]').text()).toContain("2 kB");
    expect(w.find('[data-test="original-upload"]').text()).toBe("Replace PDF");
  });

  it("rejects non-PDF and oversized files before uploading", async () => {
    const fetch = mockFetchRoutes({});
    const { w } = mountPanel(received());
    await pick(w, pdfFile(10, "scan.png", "image/png"));
    expect(w.find('[data-test="original-error"]').text()).toBe("The file is not a PDF.");
    await pick(w, pdfFile(MAX_ORIGINAL_BYTES + 1));
    expect(w.find('[data-test="original-error"]').text()).toBe("The file is too large (20 MB at most).");
    expect(fetch).not.toHaveBeenCalled();
  });

  it("maps the server's too_large and invalid file answers", async () => {
    let answer = reply(413, { code: "too_large" });
    mockFetchRoutes({ "PUT /api/documents/d1/original": () => answer });
    const { w } = mountPanel(received());
    await pick(w, pdfFile());
    expect(w.find('[data-test="original-error"]').text()).toBe("The file is too large (20 MB at most).");
    answer = reply(422, { code: "validation", fields: { file: "invalid" } });
    await pick(w, pdfFile());
    expect(w.find('[data-test="original-error"]').text()).toBe("The file is not a PDF.");
    answer = reply(422, { code: "quota_exceeded", detail: "99.5 / 100 MB" });
    await pick(w, pdfFile());
    expect(w.find('[data-test="original-error"]').text()).toContain("storage quota of the space (99.5 / 100 MB)");
  });

  it("deletes after confirmation; a missing PDF on open reads as not uploaded", async () => {
    const withPdf = received({ original: { sha256: "ab", size: 10, uploadedAt: "2026-10-08T10:00:00Z" } });
    const fetch = mockFetchRoutes({
      "DELETE /api/documents/d1/original": reply(204),
      "GET /api/documents/d1": received(),
      "GET /api/documents/d1/payments": [],
      "GET /api/documents/d1/pdf": reply(404, { code: "pdf_missing" }),
    });
    vi.stubGlobal("open", vi.fn(() => null));
    const { w } = mountPanel(withPdf);
    await w.find('[data-test="original-open"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="pdf-error"]').text()).toBe("No PDF uploaded.");

    vi.stubGlobal("confirm", vi.fn(() => true));
    await w.find('[data-test="original-delete"]').trigger("click");
    await flushPromises();
    expect(calls(fetch)).toContain("DELETE /api/documents/d1/original");
    expect(w.find('[data-test="original-missing"]').exists()).toBe(true);
  });
});
