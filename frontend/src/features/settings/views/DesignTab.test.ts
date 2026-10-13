import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { calls, mockFetchRoutes, pdfReply, reply } from "@/test-utils";
import { design, MB, mountTab, stubBrowser, VERSIONS } from "./designFixtures";

describe("DesignTab", () => {
  let tab: ReturnType<typeof stubBrowser>;

  beforeEach(() => {
    i18n.global.locale.value = "en";
    tab = stubBrowser();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  const routes = (extra: Record<string, unknown> = {}) => ({
    "GET /api/pdf/design": design(),
    "GET /api/pdf/design/versions": VERSIONS,
    ...extra,
  });

  it("lists the live files with source and size; delete only for custom files", async () => {
    mockFetchRoutes(routes());
    const w = await mountTab();

    expect(w.find('[data-test="design-storage"]').text()).toBe("S3 bucket");
    const rows = w.findAll('[data-test="design-file"]');
    expect(rows.map((r) => r.findAll("td").slice(0, 3).map((td) => td.text()))).toEqual([
      ["fonts/Inter-Regular.ttf", "default", "302.7 kB"],
      ["invoice.typ", "custom", "5 kB"],
      ["logo.svg", "custom", "812 B"],
    ]);
    expect(rows.map((r) => r.find('[data-test="design-file-delete"]').exists())).toEqual([false, true, true]);
    expect(w.find('[data-test="design-help"]').text()).toContain("invoiceapp design push");
  });

  it("names the local storage", async () => {
    mockFetchRoutes(routes({ "GET /api/pdf/design": { ...design(), storage: "fs", files: [] } }));
    const w = await mountTab();
    expect(w.find('[data-test="design-storage"]').text()).toBe("local directory");
  });

  it("shows the quota as used / limit with a proportional bar", async () => {
    mockFetchRoutes(routes({ "GET /api/pdf/design": design(null, 12.5 * MB) }));
    const w = await mountTab();

    expect(w.find('[data-test="design-quota-text"]').text()).toBe("Design storage: 12.5 MB of 50 MB");
    expect(w.find('[role="progressbar"]').attributes("aria-valuenow")).toBe("25");
    expect(w.find('[data-test="design-quota-bar"]').attributes("style")).toContain("width: 25%");
    expect(w.find('[data-test="design-quota-bar"]').classes()).toContain("bg-blue-600");
  });

  it("turns the quota bar red near the limit and caps it at 100 %", async () => {
    mockFetchRoutes(routes({ "GET /api/pdf/design": design(null, 60 * MB) }));
    const w = await mountTab();
    expect(w.find('[data-test="design-quota-bar"]').attributes("style")).toContain("width: 100%");
    expect(w.find('[data-test="design-quota-bar"]').classes()).toContain("bg-red-600");
  });

  it("downloads a custom file from live and a default file from the built-in set", async () => {
    const fetch = mockFetchRoutes(routes({ "GET /api/pdf/design/file": () => pdfReply("logo.svg", "attachment") }));
    const w = await mountTab();
    const rows = w.findAll('[data-test="design-file"]');
    await rows[2]!.find('[data-test="design-file-download"]').trigger("click");
    await flushPromises();
    await rows[0]!.find('[data-test="design-file-download"]').trigger("click");
    await flushPromises();

    expect(calls(fetch).slice(2)).toEqual([
      "GET /api/pdf/design/file?path=logo.svg&source=live",
      "GET /api/pdf/design/file?path=fonts%2FInter-Regular.ttf&source=default",
    ]);
    expect(URL.createObjectURL).toHaveBeenCalledTimes(2);
  });

  it("opens the live preview per locale", async () => {
    const fetch = mockFetchRoutes(routes({ "GET /api/pdf/preview": () => pdfReply() }));
    const w = await mountTab();
    await w.find('[data-test="design-preview-en"]').trigger("click");
    await flushPromises();
    await w.find('[data-test="design-preview-cs"]').trigger("click");
    await flushPromises();

    expect(calls(fetch).slice(2)).toEqual(["GET /api/pdf/preview?locale=en", "GET /api/pdf/preview?locale=cs"]);
    expect(tab.location.href).toBe("blob:preview");
  });

  it("shows a render failure with its detail expanded", async () => {
    mockFetchRoutes(
      routes({ "GET /api/pdf/preview": reply(502, { code: "pdf_render_failed", detail: "error: expected expression\n  ┌─ invoice.typ:12:3" }) }),
    );
    const w = await mountTab();
    await w.find('[data-test="design-preview-cs"]').trigger("click");
    await flushPromises();

    const alert = w.find('[data-test="design-preview-error"]');
    expect(alert.text()).toContain("Rendering the PDF failed.");
    expect(alert.find("details").attributes("open")).toBeDefined();
    expect(alert.find("pre").text()).toBe("error: expected expression\n  ┌─ invoice.typ:12:3");
    expect(tab.close).toHaveBeenCalledOnce();
  });
});
