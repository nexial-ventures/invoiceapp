import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, type VueWrapper } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { useToastStore } from "@/stores/toast";
import { calls, mockFetchRoutes, pdfReply, reply } from "@/test-utils";
import { design, DRAFT, mountTab, stubBrowser, VERSIONS } from "./designFixtures";

async function chooseFile(w: VueWrapper, file: File) {
  const input = w.find('[data-test="design-upload-file"]');
  Object.defineProperty(input.element, "files", { value: [file], configurable: true });
  await input.trigger("change");
}

const svg = () => new File(["<svg/>"], "logo.svg", { type: "image/svg+xml" });
const text = (w: VueWrapper, test: string) => w.find(`[data-test="${test}"]`).text();

describe("DesignTab draft", () => {
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

  const routes = (extra: Record<string, unknown> = {}, draft: typeof DRAFT | null = DRAFT) => ({
    "GET /api/pdf/design": design(draft),
    "GET /api/pdf/design/versions": VERSIONS,
    ...extra,
  });

  it("lists the pending changes", async () => {
    mockFetchRoutes(routes());
    const w = await mountTab();

    const files = w.findAll('[data-test="design-draft-file"]');
    expect(files.map((f) => f.find(".font-mono").text())).toEqual(["invoice.typ", "fonts/inter.ttf"]);
    expect(files[0]!.find('[data-test="change-replaced"]').text()).toBe("replaced");
    expect(files[1]!.find('[data-test="change-added"]').text()).toBe("added");
    expect(text(w, "design-draft-deleted")).toContain("logo.svg");
    expect(text(w, "design-draft-updated")).toMatch(/^Edited .+ \(Jana\)$/);
    expect(w.find('[data-test="design-activate"]').attributes("disabled")).toBeUndefined();
  });

  it("names a deleted author of the draft", async () => {
    mockFetchRoutes(routes({ "GET /api/pdf/design": design({ ...DRAFT, updatedBy: null }) }));
    const w = await mountTab();
    expect(text(w, "design-draft-updated")).toMatch(/\(deleted user\)$/);
  });

  it("disables activate and discard for an empty draft", async () => {
    mockFetchRoutes(routes({}, null));
    const w = await mountTab();
    expect(text(w, "design-draft-empty")).toBe("The draft is empty.");
    expect(w.find('[data-test="design-activate"]').attributes("disabled")).toBeDefined();
    expect(w.find('[data-test="design-discard"]').attributes("disabled")).toBeDefined();
  });

  it("uploads the raw file under the file name, then reloads the design and clears the form", async () => {
    const fetch = mockFetchRoutes(routes({ "PUT /api/pdf/design/draft/file": DRAFT }, null));
    const w = await mountTab();
    const file = svg();
    await chooseFile(w, file);
    expect((w.find('[data-test="design-upload-path"]').element as HTMLInputElement).value).toBe("logo.svg");

    await w.find('[data-test="design-upload"]').trigger("submit");
    await flushPromises();

    expect(calls(fetch).slice(2)).toEqual(["PUT /api/pdf/design/draft/file?path=logo.svg", "GET /api/pdf/design"]);
    expect(fetch.mock.calls[2]![1]!.body).toBe(file);
    expect((w.find('[data-test="design-upload-path"]').element as HTMLInputElement).value).toBe("");
  });

  it("lowercases the path taken from a new file's name", async () => {
    const fetch = mockFetchRoutes(routes({ "PUT /api/pdf/design/draft/file": DRAFT }));
    const w = await mountTab();
    await chooseFile(w, new File(["<svg/>"], "Logo.SVG"));
    expect((w.find('[data-test="design-upload-path"]').element as HTMLInputElement).value).toBe("logo.svg");
    await w.find('[data-test="design-upload"]').trigger("submit");
    await flushPromises();
    expect(calls(fetch)[2]).toBe("PUT /api/pdf/design/draft/file?path=logo.svg");
  });

  it("keeps a typed target path when the file is picked afterwards", async () => {
    const fetch = mockFetchRoutes(routes({ "PUT /api/pdf/design/draft/file": DRAFT }));
    const w = await mountTab();
    await w.find('[data-test="design-upload-path"]').setValue("fonts/logo.svg");
    await chooseFile(w, svg());
    await w.find('[data-test="design-upload"]').trigger("submit");
    await flushPromises();
    expect(calls(fetch)[2]).toBe("PUT /api/pdf/design/draft/file?path=fonts%2Flogo.svg");
  });

  it("replace sets the target path of the live file and opens the file picker", async () => {
    const click = vi.spyOn(HTMLInputElement.prototype, "click").mockImplementation(() => {});
    mockFetchRoutes(routes());
    const w = await mountTab();
    await w.findAll('[data-test="design-file-replace"]')[1]!.trigger("click");
    expect(click).toHaveBeenCalledOnce();
    await chooseFile(w, new File(["#let x = 1"], "my-invoice.typ"));
    expect((w.find('[data-test="design-upload-path"]').element as HTMLInputElement).value).toBe("invoice.typ");
  });

  it("replace of a default file keeps its exact (mixed-case) path", async () => {
    vi.spyOn(HTMLInputElement.prototype, "click").mockImplementation(() => {});
    const fetch = mockFetchRoutes(routes({ "PUT /api/pdf/design/draft/file": DRAFT }));
    const w = await mountTab();
    await w.findAll('[data-test="design-file-replace"]')[0]!.trigger("click");
    await chooseFile(w, new File(["font"], "Other.TTF"));
    await w.find('[data-test="design-upload"]').trigger("submit");
    await flushPromises();
    expect(calls(fetch)[2]).toBe("PUT /api/pdf/design/draft/file?path=fonts%2FInter-Regular.ttf");
  });

  it.each([
    [reply(422, { code: "validation", fields: { path: "invalid" } }), "Invalid path or file extension.", null],
    [reply(422, { code: "validation", fields: { file: "too_large" } }), "The file is larger than 10 MB.", null],
    [reply(422, { code: "quota_exceeded", detail: "49.8 MB / 50 MB" }), "The design storage quota is used up.", "49.8 MB / 50 MB"],
    [reply(409, { code: "design_busy" }), "Another design change is in progress. Try again in a moment.", null],
  ])("maps an upload failure to a message", async (answer, message, detail) => {
    mockFetchRoutes(routes({ "PUT /api/pdf/design/draft/file": answer }));
    const w = await mountTab();
    await chooseFile(w, svg());
    await w.find('[data-test="design-upload"]').trigger("submit");
    await flushPromises();

    const alert = w.find('[data-test="design-upload-error"]');
    expect(alert.find("p").text()).toBe(message);
    expect(alert.find("pre").exists() ? alert.find("pre").text() : null).toBe(detail);
    expect((w.find('[data-test="design-upload-path"]').element as HTMLInputElement).value).toBe("logo.svg");
  });

  it("removes a draft file and marks a live file for deletion", async () => {
    const fetch = mockFetchRoutes(routes({ "DELETE /api/pdf/design/draft/file": DRAFT }));
    const w = await mountTab();
    await w.findAll('[data-test="design-draft-remove"]')[1]!.trigger("click");
    await flushPromises();
    await w.findAll('[data-test="design-file-delete"]')[0]!.trigger("click");
    await flushPromises();

    expect(calls(fetch).slice(2)).toEqual([
      "DELETE /api/pdf/design/draft/file?path=fonts%2Finter.ttf",
      "GET /api/pdf/design",
      "DELETE /api/pdf/design/draft/file?path=invoice.typ",
      "GET /api/pdf/design",
    ]);
  });

  it("discards the draft only after a confirm", async () => {
    const confirm = vi.fn(() => false);
    vi.stubGlobal("confirm", confirm);
    const fetch = mockFetchRoutes(routes({ "DELETE /api/pdf/design/draft": reply(204) }));
    const w = await mountTab();
    await w.find('[data-test="design-discard"]').trigger("click");
    await flushPromises();
    expect(calls(fetch)).toHaveLength(2);

    confirm.mockReturnValue(true);
    await w.find('[data-test="design-discard"]').trigger("click");
    await flushPromises();
    expect(calls(fetch).slice(2)).toEqual(["DELETE /api/pdf/design/draft", "GET /api/pdf/design"]);
    expect(useToastStore().message).toBe("Draft discarded.");
  });

  it("activates the draft and reloads the design and the history", async () => {
    const fetch = mockFetchRoutes(routes({ "POST /api/pdf/design/activate": { version: VERSIONS[0] } }));
    const w = await mountTab();
    await w.find('[data-test="design-activate"]').trigger("click");
    await flushPromises();

    expect(calls(fetch).slice(2).sort()).toEqual(["GET /api/pdf/design", "GET /api/pdf/design/versions", "POST /api/pdf/design/activate"]);
    expect(useToastStore().message).toBe("The draft is live.");
    expect(w.find('[data-test="design-draft-error"]').exists()).toBe(false);
  });

  it.each([
    [reply(422, { code: "design_invalid", detail: "en: error: unknown variable: foo" }), "The design cannot be rendered – nothing was changed.", "en: error: unknown variable: foo"],
    [reply(422, { code: "draft_empty" }), "The draft is empty – there is nothing to activate.", null],
    [reply(429, { code: "rate_limited" }), "Too many attempts. Please try again later.", null],
    [reply(409, { code: "draft_changed" }), "The draft changed during activation. Check it and activate again.", null],
    [reply(409, { code: "design_busy" }), "Another design change is in progress. Try again in a moment.", null],
  ])("shows an activation failure", async (answer, message, detail) => {
    const fetch = mockFetchRoutes(routes({ "POST /api/pdf/design/activate": answer }));
    const w = await mountTab();
    await w.find('[data-test="design-activate"]').trigger("click");
    await flushPromises();

    const alert = w.find('[data-test="design-draft-error"]');
    expect(alert.find("p").text()).toBe(message);
    expect(alert.find("pre").exists() ? alert.find("pre").text() : null).toBe(detail);
    if (detail) expect(alert.find("details").attributes("open")).toBeDefined();
    expect(calls(fetch)).toHaveLength(3);
    expect(useToastStore().message).toBeNull();
  });

  it("opens the draft preview per locale and downloads a draft file", async () => {
    const fetch = mockFetchRoutes(
      routes({ "GET /api/pdf/design/draft/preview": () => pdfReply(), "GET /api/pdf/design/file": () => pdfReply("invoice.typ", "attachment") }),
    );
    const w = await mountTab();
    expect(text(w, "design-draft-preview-cs")).toBe("Draft preview CZ");
    await w.find('[data-test="design-draft-preview-cs"]').trigger("click");
    await flushPromises();
    await w.findAll('[data-test="design-draft-download"]')[0]!.trigger("click");
    await flushPromises();

    expect(calls(fetch).slice(2)).toEqual([
      "GET /api/pdf/design/draft/preview?locale=cs",
      "GET /api/pdf/design/file?path=invoice.typ&source=draft",
    ]);
    expect(tab.location.href).toBe("blob:preview");
  });
});
