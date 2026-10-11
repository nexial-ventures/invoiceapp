import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import type { Role } from "@/features/spaces/types";
import { useToastStore } from "@/stores/toast";
import { calls, meFixture, mockFetchRoutes, reply, sentRequest } from "@/test-utils";
import { mountView } from "@/testMount";
import type { Invite, Member } from "../types";
import MembersTab from "./MembersTab.vue";

const member = (userId: string, role: Role, extra: Partial<Member> = {}): Member => ({
  userId,
  email: `${userId}@example.cz`,
  displayName: `User ${userId}`,
  role,
  joinedAt: "2026-10-01T10:00:00Z",
  isSelf: false,
  mfaEnabled: false,
  ...extra,
});
const invite = (id: string, extra: Partial<Invite> = {}): Invite => ({
  id,
  email: `${id}@example.cz`,
  role: "member",
  invitedBy: { email: "jana@example.cz", displayName: "Jana" },
  createdAt: "2026-10-01T10:00:00Z",
  expiresAt: "2026-10-08T10:00:00Z",
  ...extra,
});

const options = (w: Awaited<ReturnType<typeof mountView>>["w"], row: number) =>
  w.findAll('[data-test="member-row"]')[row]!.findAll('[data-test="member-role"] option').map((o) => o.attributes("value"));
const rowHas = (w: Awaited<ReturnType<typeof mountView>>["w"], row: number, test: string) =>
  w.findAll('[data-test="member-row"]')[row]!.find(`[data-test="${test}"]`).exists();

describe("MembersTab", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  const mountTab = (role: Role) => mountView(MembersTab, { path: "/settings/members", role });

  it("limits an admin's controls to non-owners and the roles below owner", async () => {
    mockFetchRoutes({
      "GET /api/members": [member("o1", "owner"), member("me", "admin", { isSelf: true }), member("m1", "member")],
      "GET /api/invites": [],
    });
    const { w } = await mountTab("admin");
    // The owner: plain text, no remove.
    expect(rowHas(w, 0, "member-role")).toBe(false);
    expect(w.findAll('[data-test="member-row"]')[0]!.find('[data-test="member-role-text"]').text()).toBe("Owner");
    expect(rowHas(w, 0, "member-remove")).toBe(false);
    // Oneself: role select, no remove (leave is on the account page), "(you)" marker.
    expect(options(w, 1)).toEqual(["accountant", "member", "admin"]);
    expect(rowHas(w, 1, "member-remove")).toBe(false);
    expect(w.findAll('[data-test="member-row"]')[1]!.find('[data-test="member-self"]').text()).toBe("(you)");
    expect(options(w, 2)).toEqual(["accountant", "member", "admin"]);
    expect(rowHas(w, 2, "member-remove")).toBe(true);
    // Invitations: no owner either.
    expect(w.findAll('[data-test="invite-role"] option').map((o) => o.attributes("value"))).toEqual(["accountant", "member", "admin"]);
  });

  it("lets an owner grant owner but never demote or remove the last owner", async () => {
    mockFetchRoutes({ "GET /api/members": [member("me", "owner", { isSelf: true }), member("m1", "admin")], "GET /api/invites": [] });
    let { w } = await mountTab("owner");
    expect(rowHas(w, 0, "member-role")).toBe(false);
    expect(options(w, 1)).toEqual(["accountant", "member", "admin", "owner"]);
    expect(w.findAll('[data-test="invite-role"] option').map((o) => o.attributes("value"))).toContain("owner");

    mockFetchRoutes({ "GET /api/members": [member("me", "owner", { isSelf: true }), member("o2", "owner")], "GET /api/invites": [] });
    ({ w } = await mountTab("owner"));
    expect(options(w, 0)).toEqual(["accountant", "member", "admin", "owner"]);
    expect(rowHas(w, 1, "member-remove")).toBe(true);
  });

  it("changes a role and shows a refused change with its reason", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/members": [member("me", "owner", { isSelf: true }), member("m1", "member")],
      "GET /api/invites": [],
      "PUT /api/members/m1": (b: unknown) => member("m1", (b as { role: Role }).role),
    });
    const { w } = await mountTab("owner");
    const select = () => w.findAll('[data-test="member-role"]')[0]!;
    await select().setValue("admin");
    await flushPromises();
    expect(sentRequest(fetch, 2)).toMatchObject({ url: "/api/members/m1", method: "PUT", body: { role: "admin" } });
    expect(useToastStore().message).toBe("The role is changed.");

    mockFetchRoutes({ "PUT /api/members/m1": reply(422, { code: "validation", fields: { role: "too_high" } }) });
    await select().setValue("owner");
    await flushPromises();
    expect(w.find('[data-test="members-error"]').text()).toBe(i18n.global.t("validation.too_high"));
    expect((select().element as HTMLSelectElement).value).toBe("admin");
  });

  it("confirms changing one's own role and reloads the session after it", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/members": [member("me", "owner", { isSelf: true }), member("o2", "owner")],
      "GET /api/invites": [],
      "PUT /api/members/me": member("me", "admin", { isSelf: true }),
      "GET /api/auth/me": meFixture("admin"),
    });
    vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
    const { w, session } = await mountTab("owner");
    const select = () => w.findAll('[data-test="member-row"]')[0]!.find('[data-test="member-role"]');
    await select().setValue("admin");
    await flushPromises();
    expect(calls(fetch)).toHaveLength(2);
    expect((select().element as HTMLSelectElement).value).toBe("owner");
    await select().setValue("admin");
    await flushPromises();
    expect(calls(fetch).slice(2)).toEqual(["PUT /api/members/me", "GET /api/auth/me"]);
    expect(session.role).toBe("admin");
  });

  it("removes a member after confirmation and explains a 409 last_owner", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/members": [member("me", "owner", { isSelf: true }), member("o2", "owner"), member("m1", "member")],
      "GET /api/invites": [],
      "DELETE /api/members/m1": reply(204),
      "DELETE /api/members/o2": reply(409, { code: "last_owner" }),
    });
    vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValue(true);
    const { w } = await mountTab("owner");
    const remove = (row: number) => w.findAll('[data-test="member-row"]')[row]!.find('[data-test="member-remove"]').trigger("click");
    await remove(2);
    await flushPromises();
    expect(calls(fetch)).toHaveLength(2);
    await remove(2);
    await flushPromises();
    expect(calls(fetch)[2]).toBe("DELETE /api/members/m1");
    expect(w.findAll('[data-test="member-row"]')).toHaveLength(2);
    await remove(1);
    await flushPromises();
    expect(w.find('[data-test="members-error"]').text()).toBe(i18n.global.t("errors.lastOwner"));
  });

  it("invites, shows the link with copy and the e-mail status, and lists the invitation", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/members": [member("me", "admin", { isSelf: true })],
      "GET /api/invites": [invite("old", { email: "petr@example.cz" })],
      "POST /api/invites": reply(201, { ...invite("i1", { email: "petr@example.cz", role: "accountant" }), url: "http://firma.localhost:3000/invite?token=T1", emailSent: false }),
    });
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { w } = await mountTab("admin");
    await w.find('[data-test="invite-email"]').setValue(" petr@example.cz ");
    await w.find('[data-test="invite-role"]').setValue("accountant");
    await w.find('[data-test="invites"] form').trigger("submit");
    await flushPromises();
    expect(sentRequest(fetch, 2).body).toEqual({ email: "petr@example.cz", role: "accountant", locale: "en" });
    expect((w.find('[data-test="invite-url"]').element as HTMLInputElement).value).toBe("http://firma.localhost:3000/invite?token=T1");
    expect(w.find('[data-test="invite-email-status"]').text()).toContain("E-mail not sent (SMTP is not configured)");
    await w.find('[data-test="invite-copy"]').trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("http://firma.localhost:3000/invite?token=T1");
    // The replaced invitation for the same e-mail is gone.
    expect(w.findAll('[data-test="invite-row"]')).toHaveLength(1);
    expect(w.find('[data-test="invite-row"]').text()).toContain("Accountant");
  });

  it("shows invite validation at the fields", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/members": [member("me", "admin", { isSelf: true })],
      "GET /api/invites": [],
      "POST /api/invites": reply(422, { code: "validation", fields: { email: "already_member", role: "too_high" } }),
    });
    const { w } = await mountTab("admin");
    await w.find('[data-test="invites"] form').trigger("submit");
    expect(calls(fetch)).toHaveLength(2);
    expect(w.find("#invite-email-error").text()).toBe(i18n.global.t("validation.required"));
    await w.find('[data-test="invite-email"]').setValue("jana@example.cz");
    await w.find('[data-test="invites"] form').trigger("submit");
    await flushPromises();
    expect(w.find("#invite-email-error").text()).toBe("This user is already a member of the space.");
    expect(w.find("#invite-role-error").text()).toBe(i18n.global.t("validation.too_high"));
  });

  it("resends an invitation by e-mail (no link shown), and revokes one after confirmation", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/members": [member("me", "owner", { isSelf: true })],
      "GET /api/invites": [invite("i1"), invite("i2")],
      "POST /api/invites/i1/resend": { ...invite("i1", { expiresAt: "2026-10-17T10:00:00Z" }), emailSent: true },
      "DELETE /api/invites/i2": reply(204),
    });
    vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValue(true);
    const { w } = await mountTab("owner");
    await w.findAll('[data-test="invite-resend"]')[0]!.trigger("click");
    await flushPromises();
    expect(w.find('[data-test="invite-url"]').exists()).toBe(false);
    expect(w.find('[data-test="invite-copy"]').exists()).toBe(false);
    expect(w.find('[data-test="invite-email-status"]').text()).toBe("The link was sent by e-mail to i1@example.cz.");

    await w.findAll('[data-test="invite-revoke"]')[1]!.trigger("click");
    await flushPromises();
    expect(calls(fetch)).toHaveLength(3);
    await w.findAll('[data-test="invite-revoke"]')[1]!.trigger("click");
    await flushPromises();
    expect(calls(fetch)[3]).toBe("DELETE /api/invites/i2");
    expect(w.findAll('[data-test="invite-row"]')).toHaveLength(1);
    expect(useToastStore().message).toBe("The invitation is revoked.");
  });

  it("offers no actions on an owner invitation to an admin", async () => {
    mockFetchRoutes({
      "GET /api/members": [member("me", "admin", { isSelf: true })],
      "GET /api/invites": [invite("i1", { role: "owner" }), invite("i2", { role: "admin" })],
    });
    const { w } = await mountTab("admin");
    const rows = w.findAll('[data-test="invite-row"]');
    expect(rows[0]!.find('[data-test="invite-resend"]').exists()).toBe(false);
    expect(rows[0]!.find('[data-test="invite-revoke"]').exists()).toBe(false);
    expect(rows[1]!.find('[data-test="invite-resend"]').exists()).toBe(true);
    expect(rows[1]!.find('[data-test="invite-revoke"]').exists()).toBe(true);
  });

  it("shows a rate-limited invite and resend readably", async () => {
    mockFetchRoutes({
      "GET /api/members": [member("me", "owner", { isSelf: true })],
      "GET /api/invites": [invite("i1")],
      "POST /api/invites": reply(429, { code: "rate_limited" }),
      "POST /api/invites/i1/resend": reply(429, { code: "rate_limited" }),
    });
    const { w } = await mountTab("owner");
    await w.find('[data-test="invite-email"]').setValue("petr@example.cz");
    await w.find('[data-test="invites"] form').trigger("submit");
    await flushPromises();
    expect(w.find('[data-test="invite-error"]').text()).toBe("Too many attempts. Please try again later.");
    await w.find('[data-test="invite-resend"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="invites-error"]').text()).toBe("Too many attempts. Please try again later.");
    expect(w.find('[data-test="invite-link"]').exists()).toBe(false);
  });

  it("shows whether each member has two-factor authentication", async () => {
    mockFetchRoutes({
      "GET /api/members": [member("me", "owner", { isSelf: true, mfaEnabled: true }), member("m1", "member")],
      "GET /api/invites": [],
    });
    const { w } = await mountTab("owner");
    expect(w.findAll('[data-test="member-mfa"]').map((c) => c.text())).toEqual(["on", "off"]);
  });
});
