<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { LOCALES, setLocale, type Locale } from "@/i18n";
import type { Action } from "@/features/spaces/roles";
import { useSessionStore } from "@/stores/session";
import UserMenu from "./UserMenu.vue";

const { t, locale } = useI18n();
const route = useRoute();
const session = useSessionStore();

const LINKS: { to: string; label: string; can: Action }[] = [
  { to: "/invoices", label: "nav.invoices", can: "read" },
  { to: "/received", label: "nav.received", can: "read" },
  { to: "/contacts", label: "nav.contacts", can: "read" },
  { to: "/catalog", label: "nav.catalog", can: "read" },
  { to: "/settings", label: "nav.settings", can: "settings" },
];
// The base host has no business views; a space shows what the role may open.
const links = computed(() => (session.isSpace ? LINKS.filter((link) => session.can(link.can)) : []));
const title = computed(() => session.context?.space?.name ?? t("app.name"));

// Path prefix match: "/contacts/123" keeps "Contacts" highlighted.
function isActive(to: string): boolean {
  return route.path === to || route.path.startsWith(`${to}/`);
}

function onLocaleChange(event: Event) {
  void setLocale((event.target as HTMLSelectElement).value as Locale);
}
</script>

<template>
  <header class="border-b border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900">
    <div class="mx-auto flex max-w-5xl flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3 sm:px-6">
      <RouterLink :to="{ name: 'home' }" class="text-lg font-semibold" data-test="header-title">
        {{ title }}
      </RouterLink>

      <nav :aria-label="t('nav.main')" class="order-last flex w-full gap-4 text-sm sm:order-none sm:w-auto sm:flex-1">
        <RouterLink
          v-for="link in links"
          :key="link.to"
          :to="link.to"
          class="text-gray-600 hover:text-gray-900 dark:text-gray-400 dark:hover:text-gray-100"
          :class="{ '!text-gray-900 dark:!text-gray-100 font-medium': isActive(link.to) }"
          :data-test="`nav-${link.to.slice(1)}`"
        >
          {{ t(link.label) }}
        </RouterLink>
      </nav>

      <div class="ml-auto flex items-center gap-3">
        <label class="sr-only" for="locale-select">{{ t("locale.label") }}</label>
        <select
          id="locale-select"
          :value="locale"
          class="rounded-md border border-gray-300 bg-white px-2 py-1 text-sm uppercase dark:border-gray-700 dark:bg-gray-900"
          @change="onLocaleChange"
        >
          <option v-for="code in LOCALES" :key="code" :value="code">{{ code }}</option>
        </select>
        <UserMenu />
      </div>
    </div>
  </header>
</template>
