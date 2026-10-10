import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { leaveTo } from "@/lib/navigate";
import { useToastStore } from "@/stores/toast";
import { BASE_CONTEXT, calls, MFA_OFF, mockFetchRoutes, reply, sentRequest } from "@/test-utils";
import { mountView } from "@/testMount";
import AccountView from "../views/AccountView.vue";

vi.mock("@/lib/navigate", () => ({ leaveTo: vi.fn() }));

type Opts = { path?: string; google?: boolean; linked?: boolean; hasPassword?: boolean; mfaEnabled?: boolean };

async function mountAccount({ path = "/account", google = true, linked = false, hasPassword = true, mfaEnabled = false }: Opts = {}) {
  const r = await mountView(AccountView, { path, role: null, context: { ...BASE_CONTEXT, google }, mfaEnabled });
  r.session.patchUser({ googleLinked: linked, hasPassword });
  await flushPromises();
  return r;
}

describe("Account → Google", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    vi.mocked(leaveTo).mockClear();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  const routes = (more: Parameters<typeof mockFetchRoutes>[0] = {}) => mockFetchRoutes({ "GET /api/account/mfa": MFA_OFF, ...more });

  async function openLink(opts: Opts = {}) {
    const r = await mountAccount(opts);
    await r.w.find('[data-test="google-link-open"]').trigger("click");
    return r;
  }

  async function confirmLink(w: Awaited<ReturnType<typeof mountAccount>>["w"], password: string, code?: string) {
    await w.find('[data-test="google-link-password"]').setValue(password);
    if (code !== undefined) await w.find("#google-link-code").setValue(code);
    await w.find('[data-test="google-link-form"]').trigger("submit");
    await flushPromises();
  }

  it("links after re-confirming the password and code, then goes to the start URL", async () => {
    const start = "/api/auth/google/start?flow=abc";
    const fetch = routes({ "POST /api/account/google/link": { url: start } });
    const { w } = await openLink({ mfaEnabled: true });
    expect(w.find('[data-test="google-status"]').text()).toBe(i18n.global.t("google.account.notLinked"));
    expect(w.find('a[href*="/api/auth/google/start"]').exists()).toBe(false);

    await w.find('[data-test="google-link-form"]').trigger("submit");
    expect(w.find("#google-link-password-error").text()).toBe(i18n.global.t("validation.required"));
    expect(w.find("#google-link-code-error").text()).toBe(i18n.global.t("validation.required"));
    expect(calls(fetch)).toEqual(["GET /api/account/mfa"]);

    await confirmLink(w, "secret password", " 123456 ");
    expect(sentRequest(fetch, 1)).toMatchObject({ method: "POST", url: "/api/account/google/link", body: { password: "secret password", code: "123456" } });
    expect(leaveTo).toHaveBeenCalledWith(start);
  });

  it("sends no code without TOTP and can be cancelled", async () => {
    const fetch = routes({ "POST /api/account/google/link": { url: "/go" } });
    let { w } = await openLink();
    expect(w.find("#google-link-code").exists()).toBe(false);
    await w.find('[data-test="google-link-form"] button[type="button"]').trigger("click");
    expect(w.find('[data-test="google-link-form"]').exists()).toBe(false);

    ({ w } = await openLink());
    await confirmLink(w, "secret password");
    expect(sentRequest(fetch, 2).body).toEqual({ password: "secret password" });
  });

  it.each([
    ["password", "invalid", "Wrong password."],
    ["password", "not_set", "The account has no password set."],
    ["code", "invalid", "Wrong code."],
  ])("maps 422 %s: %s on link", async (field, reason, text) => {
    routes({ "POST /api/account/google/link": reply(422, { code: "validation", fields: { [field]: reason } }) });
    const { w } = await openLink({ mfaEnabled: true });
    await confirmLink(w, "x", "000000");
    expect(w.find(`#google-link-${field}-error`).text()).toBe(text);
    expect(leaveTo).not.toHaveBeenCalled();
  });

  it("explains a rate-limited link", async () => {
    routes({ "POST /api/account/google/link": reply(429, { code: "rate_limited" }) });
    const { w } = await openLink();
    await confirmLink(w, "x");
    expect(w.find('[data-test="google-link-error"]').text()).toBe("Too many attempts. Please try again later.");
    expect(leaveTo).not.toHaveBeenCalled();
  });

  it("hides the section when Google is off and the account is not linked", async () => {
    routes();
    const { w } = await mountAccount({ google: false });
    expect(w.find('[data-test="google-section"]').exists()).toBe(false);
  });

  it("confirms ?google=linked and shows a ?google_error", async () => {
    routes();
    let { w } = await mountAccount({ path: "/account?google=linked", linked: true });
    expect(w.find('[data-test="google-linked-now"]').text()).toBe("The Google account is linked.");
    expect(w.find('[data-test="google-status"]').text()).toBe(i18n.global.t("google.account.linked"));
    ({ w } = await mountAccount({ path: "/account?google_error=already_linked" }));
    expect(w.find('[data-test="google-error"]').text()).toBe(i18n.global.t("google.errors.already_linked"));
  });

  it("unlinks with the password and the code", async () => {
    const fetch = routes({ "DELETE /api/account/google": reply(204) });
    const { w, session } = await mountAccount({ linked: true, mfaEnabled: true });
    await w.find('[data-test="google-unlink-open"]').trigger("click");
    await w.find('[data-test="google-unlink-form"]').trigger("submit");
    expect(w.find("#google-unlink-password-error").text()).toBe(i18n.global.t("validation.required"));
    expect(w.find("#google-unlink-code-error").text()).toBe(i18n.global.t("validation.required"));

    await w.find('[data-test="google-unlink-password"]').setValue("secret password");
    await w.find("#google-unlink-code").setValue(" 123456 ");
    await w.find('[data-test="google-unlink-form"]').trigger("submit");
    await flushPromises();
    expect(sentRequest(fetch, 1)).toMatchObject({ method: "DELETE", url: "/api/account/google", body: { password: "secret password", code: "123456" } });
    expect(session.me?.user.googleLinked).toBe(false);
    expect(useToastStore().message).toBe(i18n.global.t("google.account.unlink.done"));
    expect(w.find('[data-test="google-link-open"]').exists()).toBe(true);
  });

  it.each([
    ["invalid", "Wrong password."],
    ["not_set", "The account has no password set."],
  ])("maps 422 password: %s on unlink", async (reason, text) => {
    routes({ "DELETE /api/account/google": reply(422, { code: "validation", fields: { password: reason } }) });
    const { w } = await mountAccount({ linked: true });
    await w.find('[data-test="google-unlink-open"]').trigger("click");
    await w.find('[data-test="google-unlink-password"]').setValue("x");
    await w.find('[data-test="google-unlink-form"]').trigger("submit");
    await flushPromises();
    expect(w.find("#google-unlink-password-error").text()).toBe(text);
  });
});

describe("Account without a password", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  async function setPassword(reply204OrError: unknown, mfaEnabled = false) {
    const fetch = mockFetchRoutes({ "GET /api/account/mfa": MFA_OFF, "POST /api/account/password": reply204OrError });
    const r = await mountAccount({ linked: true, hasPassword: false, mfaEnabled });
    await r.w.find('[data-test="new-password"]').setValue("new password 12");
    if (mfaEnabled) await r.w.find("#account-code").setValue("123456");
    await r.w.find('[data-test="set-password-form"]').trigger("submit");
    await flushPromises();
    return { ...r, fetch };
  }

  it("replaces every password-confirmed form by the hint", async () => {
    mockFetchRoutes({ "GET /api/account/mfa": MFA_OFF });
    const { w } = await mountAccount({ linked: true, hasPassword: false });
    expect(w.find('[data-test="current-password"]').exists()).toBe(false);
    expect(w.find('[data-test="password-form"]').exists()).toBe(false);
    expect(w.find('[data-test="google-unlink-open"]').exists()).toBe(false);
    expect(w.find('[data-test="mfa-open-enable"]').exists()).toBe(false);
    // Google section + MFA section.
    expect(w.findAll('[data-test="no-password-hint"]')).toHaveLength(2);
  });

  it("sets the first password without the current one", async () => {
    const { w, fetch, session } = await setPassword(reply(204), true);
    expect(sentRequest(fetch, 1).body).toEqual({ newPassword: "new password 12", code: "123456" });
    expect(session.hasPassword).toBe(true);
    expect(useToastStore().message).toBe(i18n.global.t("google.setPassword.done"));
    expect(w.find('[data-test="password-form"]').exists()).toBe(true);
    expect(w.find('[data-test="mfa-open-enable"]').exists()).toBe(true);
  });

  it("explains a closed 10-minute window and sends a reset link instead", async () => {
    const { w, fetch } = await setPassword(reply(422, { code: "validation", fields: { currentPassword: "required" } }));
    expect(w.find('[data-test="set-password-expired"]').text()).toBe(i18n.global.t("google.setPassword.expired"));
    mockFetchRoutes({ "POST /api/auth/password-reset": reply(204) });
    expect(calls(fetch)).toEqual(["GET /api/account/mfa", "POST /api/account/password"]);
    const form = w.find('[data-test="set-password-form"]');
    await form.find('[data-test="no-password-send"]').trigger("click");
    await flushPromises();
    expect(form.find('[data-test="no-password-sent"]').exists()).toBe(true);
  });

  it("sends the reset link to the account's e-mail", async () => {
    mockFetchRoutes({ "GET /api/account/mfa": MFA_OFF });
    const { w } = await mountAccount({ hasPassword: false });
    const fetch = mockFetchRoutes({ "POST /api/auth/password-reset": reply(204) });
    await w.find('[data-test="mfa-section"] [data-test="no-password-send"]').trigger("click");
    await flushPromises();
    expect(sentRequest(fetch).body).toEqual({ email: "jana@example.cz", locale: "en" });
  });
});
