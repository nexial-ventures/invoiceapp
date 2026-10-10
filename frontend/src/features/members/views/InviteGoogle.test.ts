import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { mockFetchRoutes, reply, SPACE_CONTEXT } from "@/test-utils";
import { mountView } from "@/testMount";
import type { InviteInfo } from "../types";
import InviteAcceptView from "./InviteAcceptView.vue";

const info: InviteInfo = { space: { slug: "firma", name: "Firma s.r.o." }, email: "jana@example.cz", role: "member", accountExists: true, requireMfa: false };

describe("InviteAcceptView with Google", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  const mountAccept = (query: string, google = true) =>
    mountView(InviteAcceptView, { path: `/invite${query}`, context: { ...SPACE_CONTEXT, google } });

  it("offers accepting with Google with the encoded token", async () => {
    mockFetchRoutes({ "GET /api/invites/accept": info });
    const { w } = await mountAccept("?token=a%2Bb%2Fc%3D");
    const link = w.find('[data-test="google-invite"]');
    expect(link.text()).toBe("Accept with Google");
    expect(link.attributes("href")).toBe("/api/auth/google/start?purpose=invite&invite=a%2Bb%2Fc%3D");
  });

  it("hides the button when Google is off", async () => {
    mockFetchRoutes({ "GET /api/invites/accept": info });
    const { w } = await mountAccept("?token=T1", false);
    expect(w.find('[data-test="google-invite"]').exists()).toBe(false);
  });

  it("shows the google_error the flow came back with", async () => {
    mockFetchRoutes({ "GET /api/invites/accept": info });
    const { w } = await mountAccept("?token=T1&google_error=email_mismatch");
    expect(w.find('[data-test="google-error"]').text()).toBe(i18n.global.t("google.errors.email_mismatch"));
  });

  it("replaces the password form by the hint on 422 password: not_set", async () => {
    mockFetchRoutes({
      "GET /api/invites/accept": info,
      "POST /api/invites/accept": reply(422, { code: "validation", fields: { password: "not_set" } }),
    });
    const { w } = await mountAccept("?token=T1");
    await w.find('[data-test="accept-password"]').setValue("anything");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(w.find("form").exists()).toBe(false);
    expect(w.find('[data-test="no-password-hint"]').text()).toContain(i18n.global.t("google.noPassword.guest"));
    expect(w.find('[data-test="google-invite"]').exists()).toBe(true);
  });

  it("shows the hint up front to the signed-in invitee without a password", async () => {
    mockFetchRoutes({ "GET /api/invites/accept": info });
    const { w, session } = await mountView(InviteAcceptView, { path: "/invite?token=T1", role: "accountant", context: { ...SPACE_CONTEXT, google: true } });
    session.patchUser({ hasPassword: false });
    await flushPromises();
    expect(w.find('[data-test="accept-password"]').exists()).toBe(false);
    expect(w.find('[data-test="no-password-send"]').exists()).toBe(true);
  });
});
