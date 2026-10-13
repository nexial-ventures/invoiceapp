import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { leaveTo } from "@/lib/navigate";
import type { Role } from "../types";
import { calls, mockFetchRoutes, reply, sentRequest } from "@/test-utils";
import { mountView } from "@/testMount";
import SpaceTab from "./SpaceTab.vue";

vi.mock("@/lib/navigate", () => ({ leaveTo: vi.fn() }));

const current = (role: Role) => ({ slug: "firma", name: "Firma s.r.o.", role, url: "http://firma.localhost:3000", requireMfa: false });

describe("SpaceTab", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    vi.mocked(leaveTo).mockClear();
  });
  afterEach(() => vi.unstubAllGlobals());

  const mountTab = (role: Role) => mountView(SpaceTab, { path: "/settings/space", role });

  it("renames the space and updates the header name", async () => {
    const fetch = mockFetchRoutes({ "GET /api/space": current("admin"), "PUT /api/space": (b: unknown) => ({ ...current("admin"), ...(b as object) }) });
    const { w, session } = await mountTab("admin");
    expect((w.find('[data-test="space-rename"]').element as HTMLInputElement).value).toBe("Firma s.r.o.");
    await w.find('[data-test="space-rename"]').setValue(" Nová firma ");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(sentRequest(fetch, 1).body).toEqual({ name: "Nová firma" });
    expect(session.context?.space?.name).toBe("Nová firma");
  });

  it("shows the storage usage against the quota, read-only", async () => {
    const limits = { storageQuotaBytes: 100 * 1024 * 1024, storageUsedBytes: 85 * 1024 * 1024 };
    mockFetchRoutes({ "GET /api/space": { ...current("admin"), limits } });
    const { w } = await mountTab("admin");
    expect(w.find('[data-test="space-storage-text"]').text()).toBe("Used 85 MB of 100 MB");
    expect(w.find('[data-test="space-storage-bar"]').attributes("style")).toContain("width: 85%");
    expect(w.find('[data-test="space-storage-bar"]').classes()).toContain("bg-amber-500");
    expect(w.find('[data-test="storage-usage"] input').exists()).toBe(false);
  });

  it("explains the MCP connection with a placeholder token", async () => {
    mockFetchRoutes({ "GET /api/space": current("admin") });
    const { w } = await mountTab("admin");
    expect(w.find('[data-test="mcp-connect"] [data-test="mcp-url"]').text()).toBe(`${window.location.origin}/api/mcp`);
    expect(w.find('[data-test="mcp-tokens-link"]').attributes("href")).toBe("/tokens");
  });

  it("hides the storage section without limits", async () => {
    mockFetchRoutes({ "GET /api/space": current("admin") });
    const { w } = await mountTab("admin");
    expect(w.find('[data-test="storage-usage"]').exists()).toBe(false);
  });

  it("offers deletion only to the owner", async () => {
    mockFetchRoutes({ "GET /api/space": current("admin") });
    let { w } = await mountTab("admin");
    expect(w.find('[data-test="delete-space"]').exists()).toBe(false);
    mockFetchRoutes({ "GET /api/space": current("owner") });
    ({ w } = await mountTab("owner"));
    expect(w.find('[data-test="delete-space"]').exists()).toBe(true);
    expect(w.find('[data-test="delete-warning"]').text()).toContain("archived");
    expect(w.find('[data-test="accountant-export"]').exists()).toBe(true);
  });

  it("deletes only after typing the slug and the password, then leaves for the base host", async () => {
    const fetch = mockFetchRoutes({ "GET /api/space": current("owner"), "DELETE /api/space": reply(204) });
    const { w } = await mountTab("owner");
    const button = () => w.find('[data-test="delete-submit"]');
    expect(button().attributes("disabled")).toBeDefined();
    await w.find('[data-test="delete-slug"]').setValue("firm");
    await w.find('[data-test="delete-password"]').setValue("my password!");
    expect(button().attributes("disabled")).toBeDefined();
    await w.find('[data-test="delete-slug"]').setValue("firma");
    expect(button().attributes("disabled")).toBeUndefined();
    await w.find('[data-test="delete-space"] form').trigger("submit");
    await flushPromises();
    expect(calls(fetch)).toEqual(["GET /api/space", "DELETE /api/space"]);
    expect(sentRequest(fetch, 1).body).toEqual({ slug: "firma", password: "my password!" });
    expect(leaveTo).toHaveBeenCalledWith("http://localhost:3000");
  });

  it("shows a wrong password and stays", async () => {
    mockFetchRoutes({ "GET /api/space": current("owner"), "DELETE /api/space": reply(422, { code: "validation", fields: { password: "invalid" } }) });
    const { w } = await mountTab("owner");
    await w.find('[data-test="delete-slug"]').setValue("firma");
    await w.find('[data-test="delete-password"]').setValue("wrong");
    await w.find('[data-test="delete-space"] form').trigger("submit");
    await flushPromises();
    expect(w.find("#delete-password-error").text()).toBe("Wrong password.");
    expect(leaveTo).not.toHaveBeenCalled();
  });

  it("explains a rate-limited delete", async () => {
    mockFetchRoutes({ "GET /api/space": current("owner"), "DELETE /api/space": reply(429, { code: "rate_limited" }) });
    const { w } = await mountTab("owner");
    await w.find('[data-test="delete-slug"]').setValue("firma");
    await w.find('[data-test="delete-password"]').setValue("wrong");
    await w.find('[data-test="delete-space"] form').trigger("submit");
    await flushPromises();
    expect(w.find('[data-test="delete-error"]').text()).toBe("Too many attempts. Please try again later.");
    expect(leaveTo).not.toHaveBeenCalled();
  });
});
