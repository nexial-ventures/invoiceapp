import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { Component } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";
import type { AppContext, Role } from "@/features/spaces/types";
import { i18n } from "@/i18n";
import { useSessionStore } from "@/stores/session";
import { BASE_CONTEXT, signIn, SPACE_CONTEXT } from "@/test-utils";

const stub = { template: "<div data-test='stub' />" };
const STUB_ROUTES = [
  { path: "/", name: "home" },
  { path: "/login", name: "login" },
  { path: "/forgot", name: "forgot" },
  { path: "/register", name: "register" },
  { path: "/verify", name: "verify" },
  { path: "/account", name: "account" },
  { path: "/tokens", name: "tokens" },
  { path: "/invoices", name: "invoices" },
  { path: "/invoices/:id/edit", name: "invoice-edit" },
  { path: "/invoices/:id", name: "invoice-detail" },
  { path: "/received/:id", name: "received-detail" },
] as const;

export interface MountOptions {
  /** Route the view is mounted at (with query); its name is taken from the stub list when it matches. */
  path?: string;
  /** Signed-in space role; `null` = base-host user; `"guest"` = signed out. */
  role?: Role | null | "guest";
  context?: AppContext;
  /** The signed-in user has TOTP on (step-up code fields appear). */
  mfaEnabled?: boolean;
}

/** Mounts a routed view with pinia, i18n and stub routes for every link target it may use. */
export async function mountView(component: Component, { path = "/", role = "guest", context, mfaEnabled = false }: MountOptions = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const ctx = context ?? (role === null || role === "guest" ? BASE_CONTEXT : SPACE_CONTEXT);
  if (role === "guest") {
    const session = useSessionStore();
    session.applyBoot({ status: "ok", context: structuredClone(ctx) });
    session.loaded = true;
  } else {
    signIn(role, ctx).setMfaEnabled(mfaEnabled);
  }
  const bare = path.split("?")[0];
  const own = STUB_ROUTES.find((r) => r.path === bare);
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: bare!, name: own?.name ?? "view", component },
      ...STUB_ROUTES.filter((r) => r.path !== bare).map((r) => ({ ...r, component: stub })),
    ],
  });
  await router.push(path);
  const w = mount({ template: "<RouterView />" }, { global: { plugins: [pinia, i18n, router] }, attachTo: document.body });
  await flushPromises();
  return { w, router, session: useSessionStore() };
}
