import { loadLocale } from "@/i18n";

// The app loads the active locale before mounting; tests switch freely, so load both.
await Promise.all([loadLocale("cs"), loadLocale("en")]);
