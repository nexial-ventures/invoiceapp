import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, RouterLinkStub, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import { useToastStore } from "@/stores/toast";
import { calls, mockFetchRoutes, reply } from "@/test-utils";
import { emailSettings, logEntry, prefill } from "../testData";
import SendEmailDialog from "./SendEmailDialog.vue";

async function mountDialog() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const w = mount(SendEmailDialog, { props: { documentId: "d1" }, global: { plugins: [pinia, i18n], stubs: { RouterLink: RouterLinkStub } } });
  await flushPromises();
  return w;
}

const chips = (w: VueWrapper, field: string) => w.findAll(`[data-test="recipients-email-${field}"] [data-test="chip"]`).map((c) => c.text().replace("×", "").trim());

async function typeAddress(w: VueWrapper, field: string, text: string) {
  const input = w.find(`#email-${field}`);
  await input.setValue(text);
  await input.trigger("keydown", { key: "Enter" });
}

describe("SendEmailDialog", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("prefills recipients, text and attachments", async () => {
    mockFetchRoutes({ "GET /api/settings/email": emailSettings(), "GET /api/documents/d1/email": prefill() });
    const w = await mountDialog();

    expect(chips(w, "to")).toEqual(["buyer@acme.cz"]);
    expect(chips(w, "bcc")).toEqual(["me@firma.cz"]);
    expect((w.find('[data-test="bcc-to-me"]').element as HTMLInputElement).checked).toBe(true);
    expect((w.find('[data-test="email-subject"]').element as HTMLInputElement).value).toBe("Faktura 20260001 – Firma");
    expect((w.find('[data-test="email-locale"]').element as HTMLSelectElement).value).toBe("cs");
    expect((w.find('[data-test="attach-pdf"]').element as HTMLInputElement).checked).toBe(true);
    expect((w.find('[data-test="attach-isdoc"]').element as HTMLInputElement).checked).toBe(true);
    expect(w.text()).toContain("20260001.isdoc");
  });

  it("toggles the company e-mail in Bcc and keeps the checkbox in sync with the chips", async () => {
    mockFetchRoutes({ "GET /api/settings/email": emailSettings(), "GET /api/documents/d1/email": prefill() });
    const w = await mountDialog();
    const box = w.find('[data-test="bcc-to-me"]');

    await box.setValue(false);
    expect(chips(w, "bcc")).toEqual([]);
    await box.setValue(true);
    expect(chips(w, "bcc")).toEqual(["me@firma.cz"]);
    await w.find('[data-test="recipients-email-bcc"] [data-test="chip-remove"]').trigger("click");
    expect((box.element as HTMLInputElement).checked).toBe(false);
  });

  it("hides the Bcc-to-me toggle without a company e-mail", async () => {
    mockFetchRoutes({ "GET /api/settings/email": emailSettings({ replyTo: null }), "GET /api/documents/d1/email": prefill({ bcc: [] }) });
    const w = await mountDialog();
    expect(w.find('[data-test="bcc-to-me"]').exists()).toBe(false);
  });

  it("switches the locale, asking first when the text was edited", async () => {
    const replies = [prefill(), prefill({ locale: "en", subject: "Invoice 20260001", body: "Hello" }), prefill({ subject: "Faktura znovu" })];
    const fetch = mockFetchRoutes({ "GET /api/settings/email": emailSettings(), "GET /api/documents/d1/email": () => replies.shift() });
    const confirm = vi.fn(() => false);
    vi.stubGlobal("confirm", confirm);
    const w = await mountDialog();
    await typeAddress(w, "cc", "boss@acme.cz");

    // Unedited: no question; recipients stay as edited.
    await w.find('[data-test="email-locale"]').setValue("en");
    await flushPromises();
    expect(confirm).not.toHaveBeenCalled();
    expect((w.find('[data-test="email-subject"]').element as HTMLInputElement).value).toBe("Invoice 20260001");
    expect(chips(w, "cc")).toEqual(["boss@acme.cz"]);

    // Edited + declined: nothing is fetched, the select goes back.
    await w.find('[data-test="email-subject"]').setValue("My subject");
    await w.find('[data-test="email-locale"]').setValue("cs");
    await flushPromises();
    expect(confirm).toHaveBeenCalledOnce();
    expect((w.find('[data-test="email-locale"]').element as HTMLSelectElement).value).toBe("en");
    expect((w.find('[data-test="email-subject"]').element as HTMLInputElement).value).toBe("My subject");

    confirm.mockReturnValue(true);
    await w.find('[data-test="email-locale"]').setValue("cs");
    await flushPromises();
    expect((w.find('[data-test="email-subject"]').element as HTMLInputElement).value).toBe("Faktura znovu");
    expect(calls(fetch).filter((c) => c.includes("/email"))).toEqual([
      "GET /api/settings/email",
      "GET /api/documents/d1/email",
      "GET /api/documents/d1/email?locale=en",
      "GET /api/documents/d1/email?locale=cs",
    ]);
  });

  it("sends the edited message and reports success", async () => {
    let sent: unknown;
    mockFetchRoutes({
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill(),
      "POST /api/documents/d1/email": (body: unknown) => ((sent = body), logEntry()),
    });
    const w = await mountDialog();
    await typeAddress(w, "to", "second@acme.cz, Third Party s.r.o. <third@acme.cz>");
    await w.find('[data-test="email-body"]').setValue("Hi");
    await w.find('[data-test="attach-isdoc"]').setValue(false);
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();

    expect(sent).toEqual({
      to: ["buyer@acme.cz", "second@acme.cz", "Third Party s.r.o. <third@acme.cz>"],
      cc: [],
      bcc: ["me@firma.cz"],
      subject: "Faktura 20260001 – Firma",
      body: "Hi",
      attachPdf: true,
      attachIsdoc: false,
    });
    expect(w.emitted("sent")).toHaveLength(1);
    expect(useToastStore().message).toBe("E-mail sent to buyer@acme.cz, second@acme.cz, Third Party s.r.o. <third@acme.cz>.");
  });

  it("requires a recipient before sending", async () => {
    const fetch = mockFetchRoutes({ "GET /api/settings/email": emailSettings(), "GET /api/documents/d1/email": prefill({ to: [] }) });
    const w = await mountDialog();
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="field-error"]').text()).toBe("This field is required.");
    expect(calls(fetch)).not.toContain("POST /api/documents/d1/email");
  });

  it("marks the addresses the server rejected", async () => {
    mockFetchRoutes({
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill({ to: ["buyer@acme.cz", "broken@"] }),
      "POST /api/documents/d1/email": reply(422, { code: "validation", fields: { "to.1": "invalid", subject: "invalid" } }),
    });
    const w = await mountDialog();
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();

    expect(w.find('[data-test="email-error"]').text()).toBe("The form contains errors.");
    expect(w.findAll('[data-test="chip-error"]').map((p) => p.text())).toEqual(["broken@: Invalid value."]);
    expect(w.find('[data-test="email-subject"]').classes()).toContain("input-error");
    // Editing the list drops its stale per-index reasons.
    await w.find('[data-test="recipients-email-to"] [data-test="chip-remove"]').trigger("click");
    expect(w.find('[data-test="chip-error"]').exists()).toBe(false);
  });

  it("shows the SMTP error with its detail", async () => {
    mockFetchRoutes({
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill(),
      "POST /api/documents/d1/email": reply(502, { code: "smtp_failed", detail: "550 5.1.1 mailbox unavailable" }),
    });
    const w = await mountDialog();
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();

    const alert = w.find('[data-test="email-error"]');
    expect(alert.text()).toContain("Sending the e-mail failed");
    expect(alert.find("pre").text()).toBe("550 5.1.1 mailbox unavailable");
    expect(w.emitted("sent")).toBeUndefined();
    expect(w.emitted("attempted")).toHaveLength(1);
  });

  it("names the e-mail limit and when it frees up", async () => {
    mockFetchRoutes({
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill(),
      "POST /api/documents/d1/email": new Response(JSON.stringify({ code: "rate_limited" }), {
        status: 429,
        headers: { "Content-Type": "application/json", "Retry-After": "1500" },
      }),
    });
    const w = await mountDialog();
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="email-error"]').text()).toContain("The e-mail limit (per hour or per day");
    expect(w.find('[data-test="email-error"]').text()).toContain("Try again in 25 min.");
  });

  it("cannot send while e-mail is disabled", async () => {
    mockFetchRoutes({
      "GET /api/settings/email": emailSettings({ disabled: true }),
      "GET /api/documents/d1/email": prefill({ disabled: true }),
    });
    const w = await mountDialog();
    expect(w.find('[data-test="email-disabled"]').text()).toContain("temporarily disabled");
    expect(w.find('[data-test="email-send"]').attributes("disabled")).toBeDefined();
  });

  it.each([
    ["smtp_not_configured", 503, "E-mail sending is not configured"],
    ["email_disabled", 503, "Sending e-mails is temporarily disabled"],
    ["pdf_render_failed", 502, "Rendering the PDF failed."],
    ["pdf_unavailable", 503, "The PDF service is unavailable."],
    ["storage_unavailable", 503, "The file storage is unavailable."],
  ])("translates %s", async (code, status, text) => {
    mockFetchRoutes({
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill(),
      "POST /api/documents/d1/email": reply(status, { code }),
    });
    const w = await mountDialog();
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="email-error"]').text()).toContain(text);
  });

  it("shows a template error of the prefill with its line and a link to fix it", async () => {
    i18n.global.locale.value = "cs";
    mockFetchRoutes({
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": reply(422, {
        code: "template_invalid",
        fields: { body: "template_invalid" },
        detail: "without contact: line 3: undefined value",
      }),
    });
    const w = await mountDialog();
    const alert = w.find('[data-test="email-error"]');
    expect(alert.find("p").text()).toBe("Šablona e-mailu obsahuje chybu.");
    expect(alert.find("pre").text()).toMatch(/^Na dokladu bez kontaktu: řádek 3: undefined value\nZjednodušený doklad/);
    expect(alert.find('[data-test="template-fix"]').text()).toBe("Opravte šablonu v Nastavení → E-mail.");
    expect(w.findComponent(RouterLinkStub).props("to")).toEqual({ name: "settings-email" });
    expect(w.emitted("attempted")).toBeUndefined();
    expect(w.find('[data-test="email-send"]').attributes("disabled")).toBeDefined();
  });

  it("disables sending when SMTP is not configured and the PDF when unavailable", async () => {
    mockFetchRoutes({
      "GET /api/settings/email": emailSettings({ configured: false }),
      "GET /api/documents/d1/email": prefill({ configured: false, attachments: { pdf: { available: false, filename: "X.pdf" }, isdoc: { available: true, filename: "X.isdoc" } } }),
    });
    const w = await mountDialog();
    expect(w.find('[data-test="email-not-configured"]').exists()).toBe(true);
    expect(w.find('[data-test="email-send"]').attributes("disabled")).toBeDefined();
    const pdf = w.find('[data-test="attach-pdf"]').element as HTMLInputElement;
    expect([pdf.checked, pdf.disabled]).toEqual([false, true]);
  });
});
