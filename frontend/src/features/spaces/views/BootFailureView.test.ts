import { describe, expect, it } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createI18n } from "vue-i18n";
import { i18n } from "@/i18n";
import { useSessionStore } from "@/stores/session";
import BootFailureView from "./BootFailureView.vue";

function mountWith(boot: Parameters<ReturnType<typeof useSessionStore>["applyBoot"]>[0], plugin = i18n) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSessionStore().applyBoot(boot);
  return mount(BootFailureView, { global: { plugins: [pinia, plugin] } });
}

describe("BootFailureView", () => {
  it("shows 'space not found' with a link to the base host", async () => {
    i18n.global.locale.value = "cs";
    const w = mountWith({ status: "not_found", baseUrl: "https://invoiceapp.cz" });
    await flushPromises();
    expect(w.find("h1").text()).toBe("Space nenalezen");
    expect(w.find('[data-test="base-link"]').attributes("href")).toBe("https://invoiceapp.cz");
  });

  it("shows the unreachable message otherwise", () => {
    const w = mountWith({ status: "error" });
    expect(w.find('[data-test="boot-error"]').exists()).toBe(true);
    expect(w.find('[data-test="boot-reload"]').exists()).toBe(false);
  });

  it("still offers a reload when no locale messages could be loaded", () => {
    const empty = createI18n({ legacy: false, locale: "cs", messages: {}, missingWarn: false, fallbackWarn: false });
    const w = mountWith({ status: "error" }, empty);
    expect(w.find("h1").text()).toContain("The app failed to load");
    expect(w.find('[data-test="boot-reload"]').exists()).toBe(true);
  });
});
