import { createI18n } from "vue-i18n";
import { loadOnce } from "@/lib/loadOnce";
import { readStorage, writeStorage } from "@/lib/storage";
import type { MessageTree } from "./namespaces";

export const LOCALES = ["cs", "en"] as const;
export type Locale = (typeof LOCALES)[number];

export const LOCALE_STORAGE_KEY = "invoice.locale";
const DEFAULT_LOCALE: Locale = "cs";

function isLocale(value: string | null): value is Locale {
  return (LOCALES as readonly string[]).includes(value ?? "");
}

const stored = readStorage(LOCALE_STORAGE_KEY);
const initial: Locale = isLocale(stored) ? stored : DEFAULT_LOCALE;
/** The last locale asked for: a slower earlier switch must not override a later one. */
let requested: Locale = initial;

/**
 * Messages are precompiled at build time (`@intlify/unplugin-vue-i18n`) and split into one chunk
 * per locale: `main.ts` loads the active one before mounting, the other loads on the first switch.
 */
const bundles: Record<Locale, () => Promise<{ default: MessageTree }>> = {
  cs: () => import("./messages/cs"),
  en: () => import("./messages/en"),
};

export const i18n = createI18n({
  legacy: false,
  locale: initial,
  fallbackLocale: DEFAULT_LOCALE,
  messages: {},
});

const loaders = Object.fromEntries(
  LOCALES.map((locale) => [
    locale,
    loadOnce(
      async () => i18n.global.setLocaleMessage(locale, (await bundles[locale]()).default as never),
      () => i18n.global.availableLocales.includes(locale),
    ),
  ]),
) as Record<Locale, () => Promise<void>>;

/** Loads a locale's messages once (concurrent callers share the load; a failure is retried). */
export const loadLocale = (locale: Locale): Promise<void> => loaders[locale]();

/**
 * Loads the messages of the locale the app starts in (before mount); if that chunk fails (e.g. a
 * stale cached `index.html` after a redeploy) falls back to the default locale. False when no
 * messages could be loaded at all: the app still mounts, on the boot error page.
 */
export async function loadInitialLocale(): Promise<boolean> {
  const first = i18n.global.locale.value as Locale;
  try {
    await loadLocale(first);
    return true;
  } catch {
    if (first === DEFAULT_LOCALE) return false;
  }
  try {
    await loadLocale(DEFAULT_LOCALE);
    i18n.global.locale.value = DEFAULT_LOCALE;
    requested = DEFAULT_LOCALE;
    return true;
  } catch {
    return false;
  }
}

/** Switches once the locale's messages are loaded; rejects (locale unchanged) when they fail. */
export async function setLocale(locale: Locale): Promise<void> {
  requested = locale;
  await loadLocale(locale);
  if (requested !== locale) return;
  i18n.global.locale.value = locale;
  writeStorage(LOCALE_STORAGE_KEY, locale);
  document.documentElement.lang = locale;
}
