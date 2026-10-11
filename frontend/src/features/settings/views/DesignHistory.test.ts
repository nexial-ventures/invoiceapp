import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { useToastStore } from "@/stores/toast";
import { calls, mockFetchRoutes, reply } from "@/test-utils";
import { design, mountTab, stubBrowser, VERSIONS } from "./designFixtures";

describe("DesignTab history", () => {
  let confirm: ReturnType<typeof vi.fn<(message?: string) => boolean>>;

  beforeEach(() => {
    i18n.global.locale.value = "en";
    stubBrowser();
    confirm = vi.fn(() => true);
    vi.stubGlobal("confirm", confirm);
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

  it("lists the versions with kind, author, file count and the current marker", async () => {
    mockFetchRoutes(routes());
    const w = await mountTab();

    const rows = w.findAll('[data-test="design-version"]');
    expect(rows.map((r) => r.findAll("td").slice(1, 5).map((td) => td.text()).filter((_, i) => i !== 2))).toEqual([
      ["activation", "Jana", "2"],
      ["default design", "petr@example.cz", "0"],
      ["activation", "deleted user", "1"],
    ]);
    expect(rows[0]!.find('[data-test="design-version-current"]').text()).toBe("current");
    expect(rows.map((r) => r.find('[data-test="design-version-restore"]').exists())).toEqual([false, true, true]);
  });

  it("restores a version after a confirm and reloads", async () => {
    const fetch = mockFetchRoutes(routes({ "POST /api/pdf/design/versions/v1/restore": { version: VERSIONS[2] } }));
    const w = await mountTab();
    await w.findAll('[data-test="design-version-restore"]')[1]!.trigger("click");
    await flushPromises();

    expect(confirm).toHaveBeenCalledWith("Restore version 1? The live files are replaced with its content.");
    expect(calls(fetch).slice(2).sort()).toEqual(["GET /api/pdf/design", "GET /api/pdf/design/versions", "POST /api/pdf/design/versions/v1/restore"]);
    expect(useToastStore().message).toBe("Version 1 restored.");
  });

  it("does nothing when the restore is not confirmed", async () => {
    confirm.mockReturnValue(false);
    const fetch = mockFetchRoutes(routes());
    const w = await mountTab();
    await w.findAll('[data-test="design-version-restore"]')[0]!.trigger("click");
    await w.find('[data-test="design-reset"]').trigger("click");
    await flushPromises();
    expect(calls(fetch)).toHaveLength(2);
  });

  it("shows the render error of a version that no longer renders", async () => {
    mockFetchRoutes(
      routes({ "POST /api/pdf/design/versions/v1/restore": reply(422, { code: "design_invalid", detail: "cs: font not found" }) }),
    );
    const w = await mountTab();
    await w.findAll('[data-test="design-version-restore"]')[1]!.trigger("click");
    await flushPromises();

    const alert = w.find('[data-test="design-history-error"]');
    expect(alert.find("p").text()).toBe("The design cannot be rendered – nothing was changed.");
    expect(alert.find("pre").text()).toBe("cs: font not found");
    expect(useToastStore().message).toBeNull();
  });

  it.each([
    ["version_incomplete", "The files of this version are missing from the storage – it cannot be restored."],
    ["design_busy", "Another design change is in progress. Try again in a moment."],
  ])("shows a restore conflict (%s)", async (code, message) => {
    mockFetchRoutes(routes({ "POST /api/pdf/design/versions/v1/restore": reply(409, { code }) }));
    const w = await mountTab();
    await w.findAll('[data-test="design-version-restore"]')[1]!.trigger("click");
    await flushPromises();
    expect(w.find('[data-test="design-history-error"]').text()).toBe(message);
  });

  it("reverts to the default design after a confirm", async () => {
    const fetch = mockFetchRoutes(routes({ "POST /api/pdf/design/reset": { version: VERSIONS[1] } }));
    const w = await mountTab();
    await w.find('[data-test="design-reset"]').trigger("click");
    await flushPromises();

    expect(confirm).toHaveBeenCalledWith(
      "Delete all custom files and use the default design? Restoring a version undoes it.",
    );
    expect(calls(fetch)).toContain("POST /api/pdf/design/reset");
    expect(useToastStore().message).toBe("The default design is in use.");
  });

  it("shows a reset failure", async () => {
    mockFetchRoutes(routes({ "POST /api/pdf/design/reset": reply(503, { code: "storage_unavailable" }) }));
    const w = await mountTab();
    await w.find('[data-test="design-reset"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="design-history-error"]').text()).toBe("The file storage is unavailable. Try again later.");
  });
});
