import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { defineComponent, h } from "vue";
import { i18n } from "@/i18n";
import { mockFetchRoutes, reply } from "@/test-utils";
import { mountView } from "@/testMount";
import DeleteSpaceSection from "./DeleteSpaceSection.vue";

const Host = defineComponent({ render: () => h(DeleteSpaceSection, { slug: "firma" }) });

describe("DeleteSpaceSection and accounts without a password", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  it("shows the hint instead of the form", async () => {
    const { w, session } = await mountView(Host, { role: "owner" });
    expect(w.find('[data-test="delete-password"]').exists()).toBe(true);
    session.patchUser({ hasPassword: false });
    await flushPromises();
    expect(w.find('[data-test="delete-password"]').exists()).toBe(false);
    expect(w.find('[data-test="no-password-hint"]').exists()).toBe(true);
  });

  it("maps 422 password: not_set", async () => {
    mockFetchRoutes({ "DELETE /api/space": reply(422, { code: "validation", fields: { password: "not_set" } }) });
    const { w } = await mountView(Host, { role: "owner" });
    await w.find('[data-test="delete-slug"]').setValue("firma");
    await w.find('[data-test="delete-password"]').setValue("x");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(w.find("#delete-password-error").text()).toBe("The account has no password set.");
  });
});
