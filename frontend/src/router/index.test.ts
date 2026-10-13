import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory } from "vue-router";
import type { Boot } from "@/boot";
import type { Role } from "@/features/spaces/types";
import { useSessionStore } from "@/stores/session";
import { BASE_CONTEXT, meFixture, mockFetchRoutes, reply, SPACE_CONTEXT } from "@/test-utils";
import { createAppRouter } from "./index";

const spaceBoot: Boot = { status: "ok", context: SPACE_CONTEXT };
const baseBoot: Boot = { status: "ok", context: BASE_CONTEXT };

function boot(b: Boot) {
  useSessionStore().applyBoot(structuredClone(b));
  return createAppRouter(createMemoryHistory(), b);
}

/** `/api/auth/me` answers as `role` (null = 401, not signed in). */
function me(role: Role | null | "base") {
  return mockFetchRoutes({ "GET /api/auth/me": role === null ? reply(401, { code: "unauthorized" }) : meFixture(role === "base" ? null : role) });
}

describe("router (space host)", () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(() => vi.unstubAllGlobals());

  it("redirects to /login without a session and keeps the target as ?return", async () => {
    me(null);
    const router = boot(spaceBoot);
    await router.push("/contacts?tab=x");
    expect(router.currentRoute.value.name).toBe("login");
    expect(router.currentRoute.value.query.return).toBe("/contacts?tab=x");
  });

  it("allows /login and /forgot without a session", async () => {
    me(null);
    const router = boot(spaceBoot);
    await router.push("/login");
    expect(router.currentRoute.value.name).toBe("login");
    await router.push("/forgot");
    expect(router.currentRoute.value.name).toBe("forgot");
  });

  it("serves the invitation page signed out and signed in alike", async () => {
    me(null);
    let router = boot(spaceBoot);
    await router.push("/invite?token=T1");
    expect(router.currentRoute.value.name).toBe("invite");
    me("member");
    setActivePinia(createPinia());
    router = boot(spaceBoot);
    await router.push("/invite?token=T1");
    expect(router.currentRoute.value.name).toBe("invite");
  });

  it("asks /api/auth/me once and sends a signed-in user away from /login", async () => {
    const fetch = me("member");
    const router = boot(spaceBoot);
    await router.push("/login");
    expect(router.currentRoute.value.name).toBe("home");
    await router.push("/contacts");
    expect(fetch).toHaveBeenCalledOnce();
  });

  it("redirects unknown paths to the overview", async () => {
    me("owner");
    const router = boot(spaceBoot);
    await router.push("/nope/deep");
    expect(router.currentRoute.value.name).toBe("home");
    expect(router.currentRoute.value.matched[0]?.path).toBe("/");
  });

  it("hides write routes from an accountant and settings from a member", async () => {
    me("accountant");
    let router = boot(spaceBoot);
    for (const path of ["/invoices/new", "/invoices/d1/edit", "/received/new", "/contacts/new", "/import/csv", "/import/isdoc", "/settings/company"]) {
      await router.push(path);
      expect(router.currentRoute.value.name, path).toBe("home");
    }
    await router.push("/invoices/d1");
    expect(router.currentRoute.value.name).toBe("invoice-detail");
    await router.push("/tokens");
    expect(router.currentRoute.value.name).toBe("tokens");

    setActivePinia(createPinia());
    me("member");
    router = boot(spaceBoot);
    await router.push("/invoices/new");
    expect(router.currentRoute.value.name).toBe("invoice-new");
    await router.push("/settings/space");
    expect(router.currentRoute.value.name).toBe("home");
    await router.push("/settings/members");
    expect(router.currentRoute.value.name).toBe("home");
  });

  it("resolves feature routes for an admin and redirects /settings to the company tab", async () => {
    me("admin");
    const router = boot(spaceBoot);
    await router.push("/settings");
    expect(router.currentRoute.value.name).toBe("settings-company");
    await router.push("/settings/space");
    expect(router.currentRoute.value.name).toBe("settings-space");
    await router.push("/settings/accounting");
    expect(router.currentRoute.value.name).toBe("settings-accounting");
    await router.push("/settings/members");
    expect(router.currentRoute.value.name).toBe("settings-members");
    await router.push("/contacts/abc");
    expect(router.currentRoute.value.name).toBe("contact-edit");
    expect(router.currentRoute.value.params.id).toBe("abc");
    await router.push("/catalog");
    expect(router.currentRoute.value.name).toBe("catalog-items");
    await router.push("/invoices/new?docType=proforma");
    expect(router.currentRoute.value.name).toBe("invoice-new");
    await router.push("/import/csv?from=received");
    expect(router.currentRoute.value.name).toBe("csv-import");
    await router.push("/account");
    expect(router.currentRoute.value.name).toBe("account");
  });

  it("has no base-host routes", async () => {
    me("owner");
    const router = boot(spaceBoot);
    expect(router.hasRoute("register")).toBe(false);
    expect(router.hasRoute("verify")).toBe(false);
  });
});

describe("router (base host)", () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(() => vi.unstubAllGlobals());

  it("serves the spaces hub at / and the account page to a signed-in user", async () => {
    me("base");
    const router = boot(baseBoot);
    await router.push("/");
    expect(router.currentRoute.value.name).toBe("home");
    expect(router.currentRoute.value.matched[0]?.path).toBe("/");
    await router.push("/account");
    expect(router.currentRoute.value.name).toBe("account");
    expect(router.hasRoute("tokens")).toBe(false);
    expect(router.hasRoute("invoices")).toBe(false);
  });

  it("lets a guest reach register, verify and reset, and sends / to login", async () => {
    me(null);
    const router = boot(baseBoot);
    await router.push("/register");
    expect(router.currentRoute.value.name).toBe("register");
    await router.push("/verify?token=abc");
    expect(router.currentRoute.value.name).toBe("verify");
    await router.push("/reset?token=abc");
    expect(router.currentRoute.value.name).toBe("reset");
    await router.push("/");
    expect(router.currentRoute.value.name).toBe("login");
    expect(router.currentRoute.value.query.return).toBe("/");
  });

  it("sends /register to login when registration is disabled", async () => {
    me(null);
    const router = boot({ status: "ok", context: { ...BASE_CONTEXT, registration: false } });
    await router.push("/register");
    expect(router.currentRoute.value.name).toBe("login");
  });
});

describe("router (boot failure)", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("shows the failure page on every path without asking for a session", async () => {
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    const router = boot({ status: "not_found", baseUrl: "http://localhost:3000" });
    await router.push("/invoices/d1");
    expect(router.currentRoute.value.name).toBe("boot-failure");
    expect(fetch).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
