import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import { useToastStore } from "@/stores/toast";
import { calls, mockFetchRoutes, reply } from "@/test-utils";
import { emailSettings, template } from "../testData";
import EmailTab from "./EmailTab.vue";

const templates = { templates: [template("cs", { custom: true }), template("en")] };

async function mountTab(routes: Record<string, unknown> = {}) {
  const fetch = mockFetchRoutes({ "GET /api/settings/email": emailSettings(), "GET /api/settings/email/templates": templates, ...routes });
  const pinia = createPinia();
  setActivePinia(pinia);
  const w = mount(EmailTab, { global: { plugins: [pinia, i18n] } });
  await flushPromises();
  return { w, fetch };
}

const editor = (locale: string) => `[data-test="template-editor-${locale}"]`;
// v-show without a document attached: the inline style is the visibility.
const hidden = (w: VueWrapper, locale: string) =>
  (w.find(editor(locale)).attributes("style") ?? "").includes("display: none");

describe("EmailTab", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("shows the configuration status", async () => {
    const { w } = await mountTab();
    expect(w.find('[data-test="email-configured"]').text()).toBe("Yes");
    expect(w.find('[data-test="email-from"]').text()).toBe("Firma <faktury@firma.cz>");
    expect(w.find('[data-test="email-reply-to"]').text()).toBe("me@firma.cz");
    expect(w.find('[data-test="email-config-help"]').exists()).toBe(false);
  });

  it("explains a missing configuration and blocks the test e-mail", async () => {
    const { w } = await mountTab({ "GET /api/settings/email": emailSettings({ configured: false, from: null, replyTo: null }) });
    expect(w.find('[data-test="email-configured"]').text()).toBe("No");
    expect(w.find('[data-test="email-reply-to"]').text()).toContain("Settings → Company");
    expect(w.find('[data-test="email-config-help"]').exists()).toBe(true);
    expect(w.find('[data-test="email-test-send"]').attributes("disabled")).toBeDefined();
  });

  it("shows the kill switch and blocks the test e-mail", async () => {
    const { w } = await mountTab({ "GET /api/settings/email": emailSettings({ disabled: true }) });
    expect(w.find('[data-test="email-disabled"]').text()).toContain("INVOICEAPP__EMAIL_DISABLED");
    expect(w.find('[data-test="email-test-send"]').attributes("disabled")).toBeDefined();
  });

  it("sends a test e-mail to the company or the given address", async () => {
    const bodies: unknown[] = [];
    const { w } = await mountTab({ "POST /api/settings/email/test": (b: unknown) => (bodies.push(b), reply(204)) });
    await w.find('[data-test="email-status"] form').trigger("submit");
    await flushPromises();
    await w.find('[data-test="email-test-to"]').setValue(" other@x.cz ");
    await w.find('[data-test="email-status"] form').trigger("submit");
    await flushPromises();
    expect(bodies).toEqual([{}, { to: "other@x.cz" }]);
    expect(useToastStore().message).toBe("Test e-mail sent.");
  });

  it("shows a failed test e-mail with the SMTP detail, and a 422 at the field", async () => {
    let answer = reply(502, { code: "smtp_failed", detail: "535 auth failed" });
    const { w } = await mountTab({ "POST /api/settings/email/test": () => answer });
    await w.find('[data-test="email-status"] form').trigger("submit");
    await flushPromises();
    expect(w.find('[data-test="email-test-error"]').text()).toContain("Sending the e-mail failed");
    expect(w.find('[data-test="email-test-error"] pre').text()).toBe("535 auth failed");

    answer = reply(422, { code: "validation", fields: { to: "required" } });
    await w.find('[data-test="email-status"] form').trigger("submit");
    await flushPromises();
    expect(w.find('[data-test="field-error"]').text()).toBe("This field is required.");
  });

  it("edits one template per locale and keeps edits across tabs", async () => {
    const { w } = await mountTab();
    expect([hidden(w, "cs"), hidden(w, "en")]).toEqual([false, true]);
    expect(w.find(`${editor("cs")} [data-test="template-custom"]`).exists()).toBe(true);
    await w.find(`${editor("cs")} [data-test="template-body"]`).setValue("Edited");

    await w.find('[data-test="template-tab-en"]').trigger("click");
    expect([hidden(w, "cs"), hidden(w, "en")]).toEqual([true, false]);
    expect(w.find(`${editor("en")} [data-test="template-default"]`).exists()).toBe(true);
    expect(w.find(`${editor("en")} [data-test="template-restore"]`).exists()).toBe(false);
    await w.find('[data-test="template-tab-cs"]').trigger("click");
    expect((w.find(`${editor("cs")} [data-test="template-body"]`).element as HTMLTextAreaElement).value).toBe("Edited");
    expect(w.find('[data-test="template-variables"]').text()).toContain("{{ doc.payable }}");
  });

  it("previews the edited text on the sample", async () => {
    const { w, fetch } = await mountTab({
      "POST /api/settings/email/templates/cs/preview": (b: unknown) => ({ subject: "Faktura 2026", body: `rendered ${(b as { body: string }).body}` }),
    });
    await w.find(`${editor("cs")} [data-test="template-body"]`).setValue("X");
    await w.find(`${editor("cs")} [data-test="template-preview"]`).trigger("click");
    await flushPromises();
    expect(w.find('[data-test="rendered-subject"]').text()).toBe("Faktura 2026");
    expect(w.find('[data-test="rendered-body"]').text()).toBe("rendered X");
    expect(calls(fetch)).not.toContain("PUT /api/settings/email/templates/cs");
  });

  it("saves, and shows template_invalid with its line at the field", async () => {
    let answer: unknown = reply(422, { code: "template_invalid", fields: { body: "template_invalid" }, detail: "line 3: undefined value" });
    const { w } = await mountTab({ "PUT /api/settings/email/templates/en": () => answer });
    const en = w.find(editor("en"));
    await en.find('[data-test="template-body"]').setValue("{{ doc.nope }}");
    await en.trigger("submit");
    await flushPromises();
    expect(en.find('[data-test="template-error"]').text()).toBe("The e-mail template contains an error.");
    expect(en.find('[data-test="field-error"]').text()).toBe("The template cannot be rendered.");
    expect(en.find('[data-test="template-detail"]').text()).toBe("line 3: undefined value");

    answer = template("en", { body: "{{ doc.number }}", custom: true });
    await en.find('[data-test="template-body"]').setValue("{{ doc.number }}");
    await en.trigger("submit");
    await flushPromises();
    expect(en.find('[data-test="template-error"]').exists()).toBe(false);
    expect(en.find('[data-test="template-custom"]').exists()).toBe(true);
    expect(useToastStore().message).toBe("Template saved.");
  });

  it("localizes a failure on the contactless sample and hints the guard", async () => {
    i18n.global.locale.value = "cs";
    const { w } = await mountTab({
      "POST /api/settings/email/templates/cs/preview": reply(422, {
        code: "template_invalid",
        fields: { body: "template_invalid" },
        detail: "without contact: line 12: undefined value",
      }),
    });
    await w.find(`${editor("cs")} [data-test="template-preview"]`).trigger("click");
    await flushPromises();
    expect(w.find(`${editor("cs")} [data-test="template-detail"]`).text()).toBe("Na dokladu bez kontaktu: řádek 12: undefined value");
    expect(w.find(`${editor("cs")} [data-test="template-hint"]`).text()).toContain("{% if contact %}");
  });

  it("validates the subject before saving", async () => {
    const { w, fetch } = await mountTab();
    await w.find(`${editor("cs")} [data-test="template-subject"]`).setValue("  ");
    await w.find(editor("cs")).trigger("submit");
    await flushPromises();
    expect(w.find(`${editor("cs")} [data-test="field-error"]`).text()).toBe("This field is required.");
    expect(calls(fetch).some((c) => c.startsWith("PUT"))).toBe(false);
  });

  it("restores the default after a confirm", async () => {
    const confirm = vi.fn(() => false);
    vi.stubGlobal("confirm", confirm);
    const { w, fetch } = await mountTab({ "DELETE /api/settings/email/templates/cs": template("cs", { subject: "Default" }) });
    await w.find('[data-test="template-restore"]').trigger("click");
    expect(calls(fetch).some((c) => c.startsWith("DELETE"))).toBe(false);

    confirm.mockReturnValue(true);
    await w.find('[data-test="template-restore"]').trigger("click");
    await flushPromises();
    expect((w.find(`${editor("cs")} [data-test="template-subject"]`).element as HTMLInputElement).value).toBe("Default");
    expect(w.find(`${editor("cs")} [data-test="template-default"]`).exists()).toBe(true);
    expect(useToastStore().message).toBe("Default template restored.");
  });

  it("reports unreachable storage", async () => {
    const { w } = await mountTab({ "PUT /api/settings/email/templates/cs": reply(503, { code: "storage_unavailable" }) });
    await w.find(editor("cs")).trigger("submit");
    await flushPromises();
    expect(w.find('[data-test="template-error"]').text()).toBe("The file storage is unavailable. Try again later.");
  });
});
