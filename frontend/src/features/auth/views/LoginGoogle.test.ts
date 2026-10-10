import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { BASE_CONTEXT, calls, meFixture, mockFetchRoutes, reply, sentRequest, SPACE_CONTEXT } from "@/test-utils";
import { mountView } from "@/testMount";
import { GOOGLE_ERRORS } from "../google";
import LoginView from "./LoginView.vue";
import RegisterView from "./RegisterView.vue";

const GOOGLE_BASE = { ...BASE_CONTEXT, google: true };
const GOOGLE_SPACE = { ...SPACE_CONTEXT, google: true };

describe("Sign in with Google on the sign-in pages", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  it.each([
    ["base", GOOGLE_BASE],
    ["space", GOOGLE_SPACE],
  ])("offers Google on the %s login page with the encoded return path", async (_, context) => {
    const { w } = await mountView(LoginView, { path: "/login?return=/invoices%3Fstatus%3Dpaid%26page%3D2", context });
    const link = w.find('[data-test="google-login"]');
    expect(link.text()).toBe("Sign in with Google");
    expect(link.attributes("href")).toBe("/api/auth/google/start?purpose=login&return=%2Finvoices%3Fstatus%3Dpaid%26page%3D2");
  });

  it("sends an unsafe or missing return to /", async () => {
    let { w } = await mountView(LoginView, { path: "/login?return=//evil.example", context: GOOGLE_BASE });
    expect(w.find('[data-test="google-login"]').attributes("href")).toBe("/api/auth/google/start?purpose=login&return=%2F");
    ({ w } = await mountView(LoginView, { path: "/login", context: GOOGLE_BASE }));
    expect(w.find('[data-test="google-login"]').attributes("href")).toBe("/api/auth/google/start?purpose=login&return=%2F");
  });

  it("hides the Google buttons when Google is not configured", async () => {
    let { w } = await mountView(LoginView, { path: "/login", context: BASE_CONTEXT });
    expect(w.find('[data-test="google-login"]').exists()).toBe(false);
    ({ w } = await mountView(RegisterView, { path: "/register", context: BASE_CONTEXT }));
    expect(w.find('[data-test="google-login"]').exists()).toBe(false);
  });

  it("offers Google sign-up on the register page", async () => {
    const { w } = await mountView(RegisterView, { path: "/register", context: GOOGLE_BASE });
    const link = w.find('[data-test="google-login"]');
    expect(link.text()).toBe("Sign up with Google");
    expect(link.attributes("href")).toBe("/api/auth/google/start?purpose=login&return=%2F");
  });

  it.each([...GOOGLE_ERRORS, "something_new"])("shows google_error=%s as a readable message", async (code) => {
    for (const locale of ["cs", "en"] as const) {
      i18n.global.locale.value = locale;
      const { w } = await mountView(LoginView, { path: `/login?google_error=${code}`, context: GOOGLE_BASE });
      const key = code === "something_new" ? "google.errors.unknown" : `google.errors.${code}`;
      expect(w.find('[data-test="google-error"]').text()).toBe(i18n.global.t(key));
    }
  });

  it("shows no Google error without the query parameter", async () => {
    const { w } = await mountView(LoginView, { path: "/login", context: GOOGLE_BASE });
    expect(w.find('[data-test="google-error"]').exists()).toBe(false);
  });

  it("opens the code step directly on ?mfa=1 and signs in with the code", async () => {
    const fetch = mockFetchRoutes({
      "POST /api/auth/login/mfa": reply(204),
      "GET /api/auth/me": meFixture("member", true, true),
    });
    const { w, router, session } = await mountView(LoginView, { path: "/login?mfa=1", context: GOOGLE_SPACE });
    expect(w.find('[data-test="login-code-step"]').exists()).toBe(true);
    expect(w.find('[data-test="login-password"]').exists()).toBe(false);
    expect(w.find('[data-test="google-login"]').exists()).toBe(false);
    await w.find('[data-test="login-code"]').setValue("123456");
    await w.find('[data-test="login-code-step"]').trigger("submit");
    await flushPromises();
    expect(calls(fetch)).toEqual(["POST /api/auth/login/mfa", "GET /api/auth/me"]);
    expect(sentRequest(fetch).body).toEqual({ code: "123456" });
    expect(session.me).not.toBeNull();
    expect(router.currentRoute.value.path).toBe("/");
  });
});
