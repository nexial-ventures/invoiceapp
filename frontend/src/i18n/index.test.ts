import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const CS = "@/i18n/messages/cs";
const EN = "@/i18n/messages/en";

/** A fresh `@/i18n` (module state: loaded locales, the requested locale) starting in `stored`. */
async function freshI18n(stored: "cs" | "en") {
  localStorage.setItem("invoice.locale", stored);
  vi.resetModules();
  return import("@/i18n");
}

const failing = () => {
  throw new Error("chunk failed");
};

describe("i18n loading", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    vi.doUnmock(CS);
    vi.doUnmock(EN);
    localStorage.clear();
  });

  it("loads the stored locale before mount", async () => {
    const { i18n, loadInitialLocale } = await freshI18n("en");
    await expect(loadInitialLocale()).resolves.toBe(true);
    expect(i18n.global.locale.value).toBe("en");
    expect(i18n.global.availableLocales).toEqual(["en"]);
  });

  it("falls back to the default locale when the stored one fails to load", async () => {
    vi.doMock(EN, failing);
    const { i18n, loadInitialLocale } = await freshI18n("en");
    await expect(loadInitialLocale()).resolves.toBe(true);
    expect(i18n.global.locale.value).toBe("cs");
    expect(i18n.global.te("app.name")).toBe(true);
  });

  it("reports false when no locale loads, so the boot error page is shown", async () => {
    vi.doMock(EN, failing);
    vi.doMock(CS, failing);
    const { loadInitialLocale } = await freshI18n("en");
    await expect(loadInitialLocale()).resolves.toBe(false);
  });

  it("keeps the last requested locale when an earlier, slower switch finishes later", async () => {
    let release = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    const real = await vi.importActual<{ default: unknown }>(EN);
    vi.doMock(EN, async () => {
      await gate;
      return real;
    });
    const { i18n, loadInitialLocale, setLocale } = await freshI18n("cs");
    await loadInitialLocale();
    const toEn = setLocale("en");
    await setLocale("cs");
    release();
    await toEn;
    expect(i18n.global.locale.value).toBe("cs");
    expect(localStorage.getItem("invoice.locale")).toBe("cs");
  });

  it("rejects a failed switch and keeps the current locale", async () => {
    vi.doMock(EN, failing);
    const { i18n, loadInitialLocale, setLocale } = await freshI18n("cs");
    await loadInitialLocale();
    await expect(setLocale("en")).rejects.toThrow();
    expect(i18n.global.locale.value).toBe("cs");
  });
});
