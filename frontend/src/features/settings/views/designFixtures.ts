import { flushPromises, mount } from "@vue/test-utils";
import { createPinia } from "pinia";
import { vi } from "vitest";
import { i18n } from "@/i18n";
import type { DesignDraft, DesignInfo, DesignVersion } from "../types";
import DesignTab from "./DesignTab.vue";

/** Shared by the Design tab tests (live files, draft, history). */
export const MB = 1024 * 1024;

export const design = (draft: DesignDraft | null = null, used = 12 * MB): DesignInfo => ({
  storage: "s3",
  files: [
    { path: "fonts/Inter-Regular.ttf", source: "default", size: 310_000 },
    { path: "invoice.typ", source: "custom", size: 5_120 },
    { path: "logo.svg", source: "custom", size: 812 },
  ],
  draft,
  quota: { used, limit: 50 * MB },
});

const by = { email: "jana@example.cz", displayName: "Jana" };
export const DRAFT: DesignDraft = {
  files: [
    { path: "invoice.typ", size: 6_000, change: "replaced" },
    { path: "fonts/inter.ttf", size: 2_048, change: "added" },
  ],
  deleted: ["logo.svg"],
  updatedAt: "2026-10-10T08:30:00Z",
  updatedBy: by,
};

/** v1's author account was deleted (`createdBy: null`). */
export const VERSIONS: DesignVersion[] = [
  { id: "v3", number: 3, kind: "activation", createdAt: "2026-10-10T09:00:00Z", createdBy: by, files: [{ path: "invoice.typ", size: 5_120, sha256: "a" }, { path: "logo.svg", size: 812, sha256: "b" }], current: true },
  { id: "v2", number: 2, kind: "default", createdAt: "2026-10-09T09:00:00Z", createdBy: { email: "petr@example.cz", displayName: "" }, files: [], current: false },
  { id: "v1", number: 1, kind: "activation", createdAt: "2026-10-08T09:00:00Z", createdBy: null, files: [{ path: "invoice.typ", size: 5_000, sha256: "c" }], current: false },
];

export async function mountTab() {
  const w = mount(DesignTab, { global: { plugins: [createPinia(), i18n] }, attachTo: document.body });
  await flushPromises();
  return w;
}

/** A fake browser tab for `openPdf`; also stubs the object URLs `saveBlob` / `openPdf` create. */
export function stubBrowser() {
  const tab = { closed: false, location: { href: "" }, close: vi.fn() };
  vi.stubGlobal("open", vi.fn(() => tab));
  vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: vi.fn(() => "blob:preview"), revokeObjectURL: vi.fn() }));
  // jsdom cannot navigate: `saveBlob` clicks a download anchor.
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  return tab;
}
