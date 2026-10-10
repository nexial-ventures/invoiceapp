import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { useToastStore } from "@/stores/toast";
import { calls, MFA_OFF, mockFetchRoutes, reply, sentRequest } from "@/test-utils";
import { mountView } from "@/testMount";
import type { MfaStatus } from "../types";
import AccountView from "../views/AccountView.vue";

type Wrapper = Awaited<ReturnType<typeof mountView>>["w"];

const CODES = Array.from({ length: 10 }, (_, i) => `aaaa${i}-bbbbb`);
const ON: MfaStatus = { enabled: true, recoveryCodesLeft: 7, requiredBy: [] };
const SETUP = { secret: "JBSWY3DPEHPK3PXP", otpauthUri: "otpauth://totp/Invoice%20(localhost):jana%40example.cz?secret=JBSWY3DPEHPK3PXP" };

describe("Account → two-factor authentication", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  const mountAccount = (mfaEnabled = false) => mountView(AccountView, { path: "/account", role: "member", mfaEnabled });
  const find = (w: Wrapper, test: string) => w.find(`[data-test="${test}"]`);

  async function fill(w: Wrapper, values: Record<string, string>, formSelector: string) {
    for (const [test, value] of Object.entries(values)) await find(w, test).setValue(value);
    await w.find(formSelector).trigger("submit");
    await flushPromises();
  }

  describe("enrolment", () => {
    it("password → QR and manual key → code → recovery codes that close only once acknowledged", async () => {
      const fetch = mockFetchRoutes({
        "GET /api/account/mfa": MFA_OFF,
        "POST /api/account/mfa/setup": SETUP,
        "POST /api/account/mfa/enable": { recoveryCodes: CODES },
      });
      const { w, session } = await mountAccount();
      expect(find(w, "mfa-status").text()).toContain("Off.");
      await find(w, "mfa-open-enable").trigger("click");
      await fill(w, { "mfa-setup-password": "my password!" }, '[data-test="mfa-enable"] form');
      expect(sentRequest(fetch, 1).body).toEqual({ password: "my password!" });

      expect(find(w, "mfa-qr").attributes("src")).toMatch(/^data:image\/svg\+xml/);
      expect((find(w, "mfa-secret").element as HTMLInputElement).value).toBe(SETUP.secret);
      await fill(w, { "mfa-enable-code": " 123456 " }, '[data-test="mfa-enable"] form');
      expect(sentRequest(fetch, 2).body).toEqual({ code: "123456" });
      expect(session.mfaEnabled).toBe(true);

      expect(w.findAll('[data-test="recovery-code"]').map((c) => c.text())).toEqual(CODES);
      expect(find(w, "recovery-close").attributes("disabled")).toBeDefined();
      await find(w, "recovery-saved").setValue(true);
      await find(w, "recovery-close").trigger("click");
      expect(find(w, "recovery-codes").exists()).toBe(false);
      expect(find(w, "mfa-status").text()).toBe("On. Recovery codes left: 10.");
      // Step-up appears on the password form right away.
      expect(find(w, "account-code").exists()).toBe(true);
      expect(calls(fetch)).toHaveLength(3);
    });

    it("copies and downloads the recovery codes", async () => {
      mockFetchRoutes({ "GET /api/account/mfa": MFA_OFF, "POST /api/account/mfa/setup": SETUP, "POST /api/account/mfa/enable": { recoveryCodes: CODES } });
      const writeText = vi.fn(async () => {});
      Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
      const createObjectURL = vi.fn((_: Blob) => "blob:codes");
      vi.stubGlobal("URL", Object.assign(URL, { createObjectURL, revokeObjectURL: vi.fn() }));
      const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
      const { w } = await mountAccount();
      await find(w, "mfa-open-enable").trigger("click");
      await fill(w, { "mfa-setup-password": "my password!" }, '[data-test="mfa-enable"] form');
      await fill(w, { "mfa-enable-code": "123456" }, '[data-test="mfa-enable"] form');

      await find(w, "recovery-copy").trigger("click");
      await flushPromises();
      expect(writeText).toHaveBeenCalledWith(CODES.join("\n"));
      await find(w, "recovery-download").trigger("click");
      expect(click).toHaveBeenCalled();
      const text = await createObjectURL.mock.calls[0]![0].text();
      expect(text).toContain("jana@example.cz");
      for (const c of CODES) expect(text).toContain(c);
    });

    it.each([
      ["invalid", "Wrong password."],
      ["not_set", "The account has no password set."],
    ])("shows 422 password: %s at its field", async (reason, text) => {
      mockFetchRoutes({ "GET /api/account/mfa": MFA_OFF, "POST /api/account/mfa/setup": reply(422, { code: "validation", fields: { password: reason } }) });
      const { w } = await mountAccount();
      await find(w, "mfa-open-enable").trigger("click");
      await fill(w, { "mfa-setup-password": "wrong" }, '[data-test="mfa-enable"] form');
      expect(w.find("#mfa-setup-password-error").text()).toBe(text);
    });

    it("keeps the code step on a wrong code and restarts from the password when the setup expired", async () => {
      const enable = vi.fn(() => reply(422, { code: "validation", fields: { code: "invalid" } }));
      mockFetchRoutes({ "GET /api/account/mfa": MFA_OFF, "POST /api/account/mfa/setup": SETUP, "POST /api/account/mfa/enable": enable });
      const { w, session } = await mountAccount();
      await find(w, "mfa-open-enable").trigger("click");
      await fill(w, { "mfa-setup-password": "my password!" }, '[data-test="mfa-enable"] form');
      await fill(w, { "mfa-enable-code": "000000" }, '[data-test="mfa-enable"] form');
      expect(w.find("#mfa-enable-code-error").text()).toBe("Wrong code.");

      enable.mockReturnValue(reply(422, { code: "validation", fields: { code: "expired" } }));
      await fill(w, { "mfa-enable-code": "123456" }, '[data-test="mfa-enable"] form');
      expect(find(w, "mfa-enable-code").exists()).toBe(false);
      expect(find(w, "mfa-setup-expired").text()).toContain("The setup has expired");
      expect(session.mfaEnabled).toBe(false);
    });

    it("hints at the spaces that require it", async () => {
      mockFetchRoutes({ "GET /api/account/mfa": { ...MFA_OFF, requiredBy: [{ slug: "firma", name: "Firma s.r.o." }] } });
      const { w } = await mountAccount();
      expect(find(w, "mfa-required-hint").text()).toContain("Firma s.r.o.");
    });
  });

  describe("when on", () => {
    it("regenerates the recovery codes with password and code and shows them once", async () => {
      const fetch = mockFetchRoutes({ "GET /api/account/mfa": ON, "POST /api/account/mfa/recovery-codes": { recoveryCodes: CODES } });
      const { w } = await mountAccount(true);
      expect(find(w, "mfa-status").text()).toBe("On. Recovery codes left: 7.");
      await find(w, "mfa-open-regenerate").trigger("click");
      await fill(w, { "mfa-regenerate-password": "my password!" }, '[data-test="mfa-regenerate"]');
      expect(w.find("#mfa-regenerate-code-error").text()).toBe(i18n.global.t("validation.required"));
      expect(calls(fetch)).toHaveLength(1);
      await fill(w, { "mfa-regenerate-code": "123456" }, '[data-test="mfa-regenerate"]');
      expect(sentRequest(fetch, 1).body).toEqual({ password: "my password!", code: "123456" });
      expect(w.findAll('[data-test="recovery-code"]')).toHaveLength(10);
      await find(w, "recovery-saved").setValue(true);
      await find(w, "recovery-close").trigger("click");
      expect(find(w, "mfa-status").text()).toBe("On. Recovery codes left: 10.");
    });

    it("warns when few recovery codes are left", async () => {
      mockFetchRoutes({ "GET /api/account/mfa": { ...ON, recoveryCodesLeft: 2 } });
      const { w } = await mountAccount(true);
      expect(find(w, "mfa-codes-low").exists()).toBe(true);
    });

    it("maps a wrong code on disable", async () => {
      mockFetchRoutes({ "GET /api/account/mfa": ON, "POST /api/account/mfa/disable": reply(422, { code: "validation", fields: { code: "invalid" } }) });
      const { w, session } = await mountAccount(true);
      await find(w, "mfa-open-disable").trigger("click");
      expect(find(w, "mfa-required-by").exists()).toBe(false);
      await fill(w, { "mfa-disable-password": "my password!", "mfa-disable-code": "000000" }, '[data-test="mfa-disable"]');
      expect(w.find("#mfa-disable-code-error").text()).toBe("Wrong code.");
      expect(session.mfaEnabled).toBe(true);
    });

    it("maps password: not_set on disable and hides the actions for an account without a password", async () => {
      mockFetchRoutes({ "GET /api/account/mfa": ON, "POST /api/account/mfa/disable": reply(422, { code: "validation", fields: { password: "not_set" } }) });
      const { w, session } = await mountAccount(true);
      await find(w, "mfa-open-disable").trigger("click");
      await fill(w, { "mfa-disable-password": "x", "mfa-disable-code": "000000" }, '[data-test="mfa-disable"]');
      expect(w.find("#mfa-disable-password-error").text()).toBe("The account has no password set.");
      await w.find('[data-test="mfa-disable"] button[type="button"]').trigger("click");
      session.patchUser({ hasPassword: false });
      await flushPromises();
      expect(find(w, "mfa-open-disable").exists()).toBe(false);
      expect(find(w, "mfa-open-regenerate").exists()).toBe(false);
      expect(w.find('[data-test="mfa-section"] [data-test="no-password-hint"]').exists()).toBe(true);
    });

    it("disables after listing the spaces that require it", async () => {
      const status = { ...ON, requiredBy: [{ slug: "jina", name: "Jiná firma" }] };
      const fetch = mockFetchRoutes({ "GET /api/account/mfa": status, "POST /api/account/mfa/disable": reply(204) });
      const { w, session, router } = await mountAccount(true);
      await find(w, "mfa-open-disable").trigger("click");
      expect(find(w, "mfa-required-by").text()).toContain("Jiná firma");
      await fill(w, { "mfa-disable-password": "my password!", "mfa-disable-code": "123456" }, '[data-test="mfa-disable"]');
      expect(sentRequest(fetch, 1).body).toEqual({ password: "my password!", code: "123456" });
      expect(find(w, "mfa-status").text()).toContain("Off.");
      expect(session.mfaEnabled).toBe(false);
      expect(find(w, "account-code").exists()).toBe(false);
      expect(useToastStore().message).toBe("Two-factor authentication is off.");
      // Another space required it, not this one: the session here stays.
      expect(router.currentRoute.value.name).toBe("account");
    });

    it("signs out when this very space requires it", async () => {
      mockFetchRoutes({ "GET /api/account/mfa": { ...ON, requiredBy: [{ slug: "firma", name: "Firma s.r.o." }] }, "POST /api/account/mfa/disable": reply(204) });
      const { w, session, router } = await mountAccount(true);
      await find(w, "mfa-open-disable").trigger("click");
      await fill(w, { "mfa-disable-password": "my password!", "mfa-disable-code": "123456" }, '[data-test="mfa-disable"]');
      expect(session.me).toBeNull();
      expect(router.currentRoute.value.name).toBe("login");
    });
  });
});
