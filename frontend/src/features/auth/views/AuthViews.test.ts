import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { calls, meFixture, mockFetchRoutes, reply, sentRequest } from "@/test-utils";
import { mountView } from "@/testMount";
import ForgotView from "./ForgotView.vue";
import LoginView from "./LoginView.vue";
import RegisterView from "./RegisterView.vue";
import ResetView from "./ResetView.vue";
import VerifyView from "./VerifyView.vue";

describe("auth views", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  describe("LoginView", () => {
    it("validates before sending", async () => {
      const fetch = mockFetchRoutes({});
      const { w } = await mountView(LoginView, { path: "/login" });
      await w.find('[data-test="login-email"]').setValue("nope");
      await w.find("form").trigger("submit");
      await flushPromises();
      expect(fetch).not.toHaveBeenCalled();
      expect(w.findAll('[data-test="field-error"]').map((e) => e.text())).toEqual([i18n.global.t("validation.invalid"), i18n.global.t("validation.required")]);
    });

    it("signs in, loads the user and follows ?return", async () => {
      const fetch = mockFetchRoutes({ "POST /api/auth/login": reply(204), "GET /api/auth/me": meFixture("member") });
      const { w, router, session } = await mountView(LoginView, { path: "/login?return=/invoices" });
      await w.find('[data-test="login-email"]').setValue(" jana@example.cz ");
      await w.find('[data-test="login-password"]').setValue("correct horse battery");
      await w.find("form").trigger("submit");
      await flushPromises();
      expect(sentRequest(fetch).body).toEqual({ email: "jana@example.cz", password: "correct horse battery" });
      expect(session.role).toBe("member");
      expect(router.currentRoute.value.path).toBe("/invoices");
    });

    it("ignores a foreign ?return", async () => {
      mockFetchRoutes({ "POST /api/auth/login": reply(204), "GET /api/auth/me": meFixture("member") });
      const { w, router } = await mountView(LoginView, { path: "/login?return=//evil.example" });
      await w.find('[data-test="login-email"]').setValue("jana@example.cz");
      await w.find('[data-test="login-password"]').setValue("x");
      await w.find("form").trigger("submit");
      await flushPromises();
      expect(router.currentRoute.value.path).toBe("/");
    });

    it.each([
      [401, "invalid_credentials", "Wrong e-mail or password."],
      [429, "rate_limited", "Too many attempts. Please try again later."],
    ])("explains %i %s", async (status, code, text) => {
      mockFetchRoutes({ "POST /api/auth/login": reply(status, { code }) });
      const { w, router } = await mountView(LoginView, { path: "/login" });
      await w.find('[data-test="login-email"]').setValue("jana@example.cz");
      await w.find('[data-test="login-password"]').setValue("wrong");
      await w.find("form").trigger("submit");
      await flushPromises();
      expect(w.find('[data-test="login-error"]').text()).toBe(text);
      expect(router.currentRoute.value.name).toBe("login");
    });

    it("titles the space and offers registration only on the base host when enabled", async () => {
      mockFetchRoutes({});
      let { w } = await mountView(LoginView, { path: "/login" });
      expect(w.find('[data-test="register-link"]').exists()).toBe(true);
      ({ w } = await mountView(LoginView, { path: "/login", context: { kind: "base", space: null, registration: false, google: false, baseUrl: "http://localhost:3000" } }));
      expect(w.find('[data-test="register-link"]').exists()).toBe(false);
      ({ w } = await mountView(LoginView, { path: "/login", context: { kind: "space", space: { slug: "firma", name: "Firma" }, registration: true, google: false, baseUrl: "http://x" } }));
      expect(w.find("h1").text()).toBe("Sign in – Firma");
      expect(w.find('[data-test="register-link"]').exists()).toBe(false);
    });
  });

  describe("RegisterView", () => {
    async function fill(w: Awaited<ReturnType<typeof mountView>>["w"], password = "twelve chars!") {
      await w.find('[data-test="register-email"]').setValue("jana@example.cz");
      await w.find('[data-test="register-name"]').setValue(" Jana ");
      await w.find('[data-test="register-password"]').setValue(password);
      await w.find("form").trigger("submit");
      await flushPromises();
    }

    it("requires a 12+ character password before sending", async () => {
      const fetch = mockFetchRoutes({});
      const { w } = await mountView(RegisterView, { path: "/register" });
      await fill(w, "short");
      expect(fetch).not.toHaveBeenCalled();
      expect(w.find("#register-password-error").text()).toBe(i18n.global.t("validation.too_short"));
    });

    it("sends the locale and shows one enumeration-safe message on 202", async () => {
      const fetch = mockFetchRoutes({ "POST /api/auth/register": reply(202) });
      const { w } = await mountView(RegisterView, { path: "/register" });
      await fill(w);
      expect(sentRequest(fetch).body).toEqual({ email: "jana@example.cz", displayName: "Jana", password: "twelve chars!", locale: "en" });
      expect(w.find('[data-test="register-done"]').text()).toBe(i18n.global.t("auth.register.done"));
      expect(w.find("form").exists()).toBe(false);
    });

    it("shows the server's 422 at the field", async () => {
      mockFetchRoutes({ "POST /api/auth/register": reply(422, { code: "validation", fields: { email: "invalid" } }) });
      const { w } = await mountView(RegisterView, { path: "/register" });
      await fill(w);
      expect(w.find("#register-email-error").text()).toBe(i18n.global.t("validation.invalid"));
      expect(w.find('[data-test="register-done"]').exists()).toBe(false);
    });
  });

  it("ForgotView always answers with the same message", async () => {
    const fetch = mockFetchRoutes({ "POST /api/auth/password-reset": reply(202) });
    const { w } = await mountView(ForgotView, { path: "/forgot" });
    await w.find('[data-test="forgot-email"]').setValue("who@example.cz");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(sentRequest(fetch).body).toEqual({ email: "who@example.cz", locale: "en" });
    expect(w.find('[data-test="forgot-sent"]').text()).toBe(i18n.global.t("auth.forgot.sent"));
  });

  describe("VerifyView", () => {
    it("confirms the token from the link", async () => {
      const fetch = mockFetchRoutes({ "POST /api/auth/verify": reply(204) });
      const { w } = await mountView(VerifyView, { path: "/verify?token=abc" });
      expect(sentRequest(fetch).body).toEqual({ token: "abc" });
      expect(w.find('[data-test="verified"]').exists()).toBe(true);
    });

    it("offers a resend for an invalid token", async () => {
      const fetch = mockFetchRoutes({
        "POST /api/auth/verify": reply(422, { code: "validation", fields: { token: "invalid" } }),
        "POST /api/auth/verify/resend": reply(202),
      });
      const { w } = await mountView(VerifyView, { path: "/verify?token=old" });
      expect(w.find('[data-test="verify-invalid"]').exists()).toBe(true);
      await w.find('[data-test="resend-email"]').setValue("jana@example.cz");
      await w.find("form").trigger("submit");
      await flushPromises();
      expect(calls(fetch)).toEqual(["POST /api/auth/verify", "POST /api/auth/verify/resend"]);
      expect(sentRequest(fetch, 1).body).toEqual({ email: "jana@example.cz", locale: "en" });
      expect(w.find('[data-test="resent"]').exists()).toBe(true);
    });

    it("without a token asks to verify, prefilled with the signed-in e-mail", async () => {
      mockFetchRoutes({});
      const { w } = await mountView(VerifyView, { path: "/verify", role: null });
      expect(w.find('[data-test="verify-needed"]').exists()).toBe(true);
      expect((w.find('[data-test="resend-email"]').element as HTMLInputElement).value).toBe("jana@example.cz");
    });
  });

  describe("ResetView", () => {
    it("sets the new password and signs this browser out", async () => {
      const fetch = mockFetchRoutes({ "POST /api/auth/password-reset/confirm": reply(204) });
      const { w, session } = await mountView(ResetView, { path: "/reset?token=t1", role: null });
      await w.find('[data-test="reset-password"]').setValue("a brand new password");
      await w.find("form").trigger("submit");
      await flushPromises();
      expect(sentRequest(fetch).body).toEqual({ token: "t1", password: "a brand new password" });
      expect(w.find('[data-test="reset-done"]').exists()).toBe(true);
      expect(session.me).toBeNull();
    });

    it("explains an invalid or missing token", async () => {
      mockFetchRoutes({ "POST /api/auth/password-reset/confirm": reply(422, { code: "validation", fields: { token: "invalid" } }) });
      let { w } = await mountView(ResetView, { path: "/reset?token=used" });
      await w.find('[data-test="reset-password"]').setValue("a brand new password");
      await w.find("form").trigger("submit");
      await flushPromises();
      expect(w.find('[data-test="reset-invalid"]').exists()).toBe(true);
      ({ w } = await mountView(ResetView, { path: "/reset" }));
      expect(w.find('[data-test="reset-invalid"]').exists()).toBe(true);
    });
  });
});
