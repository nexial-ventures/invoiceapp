import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { useSessionStore } from "@/stores/session";
import { BASE_CONTEXT, calls, meFixture, mockFetchRoutes, reply, sentRequest } from "@/test-utils";
import { mountView } from "@/testMount";
import type { Space } from "../types";
import SpacesView from "./SpacesView.vue";

const space = (slug: string, name: string, role: Space["role"] = "owner"): Space => ({ slug, name, role, url: `http://${slug}.localhost:3000`, requireMfa: false });

describe("SpacesView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  const mountHub = () => mountView(SpacesView, { path: "/", role: null });

  it("lists the memberships with their role and a link to each host", async () => {
    mockFetchRoutes({ "GET /api/spaces": [space("alfa", "Alfa"), space("beta", "Beta", "accountant")] });
    const { w } = await mountHub();
    const rows = w.findAll('[data-test="space-row"]');
    expect(rows).toHaveLength(2);
    expect(rows[1]!.text()).toContain("Accountant");
    expect(rows[1]!.find('[data-test="space-open"]').attributes("href")).toBe("http://beta.localhost:3000");
  });

  it("validates the slug live and previews {slug}.{base host}", async () => {
    mockFetchRoutes({ "GET /api/spaces": [] });
    const { w } = await mountHub();
    expect(w.find('[data-test="spaces-empty"]').exists()).toBe(true);
    const preview = () => w.find('[data-test="slug-preview"]').text();
    expect(preview()).toBe("….localhost:3000");
    expect(w.find("#space-slug-error").exists()).toBe(false);

    await w.find('[data-test="space-slug"]').setValue("Fi");
    await flushPromises();
    expect((w.find('[data-test="space-slug"]').element as HTMLInputElement).value).toBe("fi");
    expect(w.find("#space-slug-error").text()).toBe(i18n.global.t("validation.invalid"));

    await w.find('[data-test="space-slug"]').setValue("admin");
    await flushPromises();
    expect(w.find("#space-slug-error").text()).toBe(i18n.global.t("validation.reserved"));

    await w.find('[data-test="space-slug"]').setValue("firma");
    await flushPromises();
    expect(w.find("#space-slug-error").exists()).toBe(false);
    expect(preview()).toBe("firma.localhost:3000");
  });

  it("creates a space and adds it to the list by name", async () => {
    const fetch = mockFetchRoutes({ "GET /api/spaces": [space("zeta", "Zeta")], "POST /api/spaces": reply(201, space("firma", "Firma")) });
    const { w } = await mountHub();
    await w.find('[data-test="space-name"]').setValue(" Firma ");
    await w.find('[data-test="space-slug"]').setValue("firma");
    await w.find('[data-test="new-space"]').trigger("submit");
    await flushPromises();
    expect(sentRequest(fetch, 1).body).toEqual({ slug: "firma", name: "Firma" });
    expect(w.findAll('[data-test="space-row"]').map((r) => r.find("p").text())).toEqual(["Firma", "Zeta"]);
    expect((w.find('[data-test="space-slug"]').element as HTMLInputElement).value).toBe("");
  });

  it("shows a taken slug from the server until the slug changes", async () => {
    mockFetchRoutes({ "GET /api/spaces": [], "POST /api/spaces": reply(422, { code: "validation", fields: { slug: "taken" } }) });
    const { w } = await mountHub();
    await w.find('[data-test="space-name"]').setValue("Firma");
    await w.find('[data-test="space-slug"]').setValue("firma");
    await w.find('[data-test="new-space"]').trigger("submit");
    await flushPromises();
    expect(w.find("#space-slug-error").text()).toBe(i18n.global.t("validation.taken"));
    await w.find('[data-test="space-slug"]').setValue("firma2");
    await flushPromises();
    expect(w.find("#space-slug-error").exists()).toBe(false);
  });

  it("does not send an invalid form", async () => {
    const fetch = mockFetchRoutes({ "GET /api/spaces": [] });
    const { w } = await mountHub();
    await w.find('[data-test="new-space"]').trigger("submit");
    await flushPromises();
    expect(calls(fetch)).toEqual(["GET /api/spaces"]);
    expect(w.find("#space-name-error").exists()).toBe(true);
    expect(w.find("#space-slug-error").text()).toBe(i18n.global.t("validation.required"));
  });

  it("asks an unverified user to verify instead of offering the form", async () => {
    mockFetchRoutes({ "GET /api/spaces": [] });
    const { w } = await mountView(SpacesView, { path: "/", role: null });
    useSessionStore().me = meFixture(null, false);
    await flushPromises();
    expect(w.find('[data-test="unverified"]').exists()).toBe(true);
    expect(w.find('[data-test="new-space"]').exists()).toBe(false);
  });

  it("replaces the form by a note while registration (and so space creation) is off", async () => {
    mockFetchRoutes({ "GET /api/spaces": [space("alfa", "Alfa")] });
    const { w } = await mountView(SpacesView, { path: "/", role: null, context: { ...BASE_CONTEXT, registration: false } });
    expect(w.find('[data-test="new-space"]').exists()).toBe(false);
    expect(w.find('[data-test="creation-disabled"]').text()).toBe("Creating new spaces is disabled on this instance.");
    expect(w.findAll('[data-test="space-row"]')).toHaveLength(1);
  });

  it("shows a readable message when the server refuses creation", async () => {
    mockFetchRoutes({ "GET /api/spaces": [], "POST /api/spaces": reply(403, { code: "space_creation_disabled" }) });
    const { w } = await mountHub();
    await w.find('[data-test="space-name"]').setValue("Firma");
    await w.find('[data-test="space-slug"]').setValue("firma");
    await w.find('[data-test="new-space"]').trigger("submit");
    await flushPromises();
    expect(w.find('[data-test="new-space-error"]').text()).toBe(i18n.global.t("errors.spaceCreationDisabled"));
  });
});
