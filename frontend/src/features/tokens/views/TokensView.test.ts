import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import type { Role } from "@/features/spaces/types";
import { calls, mockFetchRoutes, reply, sentRequest } from "@/test-utils";
import { mountView } from "@/testMount";
import type { ApiToken } from "../types";
import TokensView from "./TokensView.vue";

const token = (id: string, extra: Partial<ApiToken> = {}): ApiToken => ({
  id,
  name: `Token ${id}`,
  prefix: "abcd1234",
  role: "member",
  createdAt: "2026-10-01T10:00:00Z",
  expiresAt: null,
  lastUsedAt: null,
  ...extra,
});

describe("TokensView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  const mountTokens = (role: Role) => mountView(TokensView, { path: "/tokens", role });

  it("lists own tokens without owner columns for a member", async () => {
    mockFetchRoutes({ "GET /api/tokens": [token("t1", { expiresAt: "2026-12-31" })] });
    const { w } = await mountTokens("member");
    const row = w.find('[data-test="token-row"]');
    expect(row.text()).toContain("inv_abcd1234_…");
    expect(row.text()).toContain("not used");
    expect(w.find('[data-test="user-column"]').exists()).toBe(false);
  });

  it("explains the MCP connection with a placeholder token below the list", async () => {
    mockFetchRoutes({ "GET /api/tokens": [] });
    const { w } = await mountTokens("accountant");
    expect(w.find('[data-test="mcp-connect"] [data-test="mcp-value"]').text()).toBe("Bearer <TOKEN>");
  });

  it("shows the owner of every token to an admin", async () => {
    mockFetchRoutes({ "GET /api/tokens": [token("t1", { user: { email: "petr@example.cz", displayName: "Petr" } })] });
    const { w } = await mountTokens("admin");
    expect(w.find('[data-test="user-column"]').exists()).toBe(true);
    expect(w.find('[data-test="token-user"]').text()).toContain("petr@example.cz");
  });

  it("creates a token with roles up to the own role and shows the secret once", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/tokens": [],
      "POST /api/tokens": reply(201, { ...token("t9", { name: "MCP", role: "member", expiresAt: "2099-01-31" }), token: "inv_abcd1234_SECRET" }),
    });
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { w } = await mountTokens("member");

    await w.find('[data-test="new-token"]').trigger("click");
    const options = w.findAll('[data-test="token-role"] option').map((o) => o.attributes("value"));
    expect(options).toEqual(["accountant", "member"]);
    await w.find('[data-test="token-name"]').setValue(" MCP ");
    await w.find('[data-test="token-role"]').setValue("member");
    await w.find('[data-test="token-expires"]').setValue("2099-01-31");
    await w.find('[data-test="token-dialog"] form').trigger("submit");
    await flushPromises();

    expect(sentRequest(fetch, 1).body).toEqual({ name: "MCP", role: "member", expiresAt: "2099-01-31" });
    expect((w.find('[data-test="token-secret"]').element as HTMLInputElement).value).toBe("inv_abcd1234_SECRET");
    await w.find('[data-test="token-copy"]').trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("inv_abcd1234_SECRET");

    expect(w.find('[data-test="token-dialog"] [data-test="mcp-connect"]').exists()).toBe(false);
    await w.find('[data-test="token-mcp-toggle"]').trigger("click");
    const guide = w.find('[data-test="token-dialog"] [data-test="mcp-connect"]');
    expect(guide.find('[data-test="mcp-value"]').text()).toBe("Bearer inv_abcd1234_SECRET");
    expect(guide.find('[data-test="mcp-command"]').text()).toContain("Authorization: Bearer inv_abcd1234_SECRET");

    await w.find('[data-test="token-done"]').trigger("click");
    expect(w.find('[data-test="token-dialog"]').exists()).toBe(false);
    expect(w.findAll('[data-test="token-row"]')).toHaveLength(1);
    expect(w.html()).not.toContain("SECRET");
  });

  it("sends no expiry when left empty and shows a too_high role at the field", async () => {
    const fetch = mockFetchRoutes({ "GET /api/tokens": [], "POST /api/tokens": reply(422, { code: "validation", fields: { role: "too_high" } }) });
    const { w } = await mountTokens("owner");
    await w.find('[data-test="new-token"]').trigger("click");
    await w.find('[data-test="token-name"]').setValue("Script");
    await w.find('[data-test="token-dialog"] form').trigger("submit");
    await flushPromises();
    expect(sentRequest(fetch, 1).body).toEqual({ name: "Script", role: "accountant", expiresAt: null });
    expect(w.find("#token-role-error").text()).toBe(i18n.global.t("validation.too_high"));
    expect(w.find('[data-test="token-secret"]').exists()).toBe(false);
  });

  it("revokes after confirmation", async () => {
    const fetch = mockFetchRoutes({ "GET /api/tokens": [token("t1"), token("t2")], "DELETE /api/tokens/t1": reply(204) });
    vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
    const { w } = await mountTokens("member");
    await w.find('[data-test="token-revoke"]').trigger("click");
    await flushPromises();
    expect(calls(fetch)).toEqual(["GET /api/tokens"]);
    await w.find('[data-test="token-revoke"]').trigger("click");
    await flushPromises();
    expect(calls(fetch)).toEqual(["GET /api/tokens", "DELETE /api/tokens/t1"]);
    expect(w.findAll('[data-test="token-row"]').map((r) => r.find("td").text())).toEqual(["Token t2"]);
  });

  describe("step-up code", () => {
    const open = async (mfaEnabled: boolean) => {
      const r = await mountView(TokensView, { path: "/tokens", role: "member", mfaEnabled });
      await r.w.find('[data-test="new-token"]').trigger("click");
      await r.w.find('[data-test="token-name"]').setValue("MCP");
      return r;
    };
    const submit = async (w: Awaited<ReturnType<typeof open>>["w"]) => {
      await w.find('[data-test="token-dialog"] form').trigger("submit");
      await flushPromises();
    };

    it("is absent without TOTP", async () => {
      mockFetchRoutes({ "GET /api/tokens": [] });
      const { w } = await open(false);
      expect(w.find('[data-test="token-code"]').exists()).toBe(false);
    });

    it("is required with TOTP, sent with the token and a wrong one is shown at the field", async () => {
      const fetch = mockFetchRoutes({ "GET /api/tokens": [], "POST /api/tokens": reply(422, { code: "validation", fields: { code: "invalid" } }) });
      const { w } = await open(true);
      await submit(w);
      expect(calls(fetch)).toEqual(["GET /api/tokens"]);
      expect(w.find("#token-code-error").text()).toBe(i18n.global.t("validation.required"));
      await w.find('[data-test="token-code"]').setValue(" 123456 ");
      await submit(w);
      expect(sentRequest(fetch, 1).body).toEqual({ name: "MCP", role: "accountant", expiresAt: null, code: "123456" });
      expect(w.find("#token-code-error").text()).toBe("Wrong code.");
      expect(w.find('[data-test="token-secret"]').exists()).toBe(false);
    });

    it("explains 403 mfa_required with a link to the account on the base host", async () => {
      mockFetchRoutes({ "GET /api/tokens": [], "POST /api/tokens": reply(403, { code: "mfa_required" }) });
      const { w } = await open(false);
      await submit(w);
      expect(w.find('[data-test="token-mfa-required"]').text()).toContain("This space requires two-factor authentication");
      expect(w.find('[data-test="token-mfa-link"]').attributes("href")).toBe("http://localhost:3000/account");
      expect(w.find('[data-test="token-error"]').exists()).toBe(false);
      expect(w.find('[data-test="token-secret"]').exists()).toBe(false);
    });
  });
});
