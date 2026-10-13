<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import { formatDate } from "@/lib/format";
import { useSessionStore } from "@/stores/session";
import { useToastStore } from "@/stores/toast";
import CreateTokenDialog from "../components/CreateTokenDialog.vue";
import McpConnectCard from "../components/McpConnectCard.vue";
import { useTokensStore } from "../store";
import type { ApiToken } from "../types";

const { t, locale } = useI18n();
const session = useSessionStore();
const store = useTokensStore();
const toast = useToastStore();
const { error, run } = useAction();
const creating = ref(false);
// Admin+ list every token of the space, with its owner.
const showUser = computed(() => session.can("viewAllTokens"));

onMounted(() => void run(store.load));

async function revoke(token: ApiToken) {
  if (!window.confirm(t("tokens.confirmRevoke", { name: token.name }))) return;
  if (await run(() => store.revoke(token.id))) toast.show(t("tokens.revoked"));
}
</script>

<template>
  <section class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <h1 class="text-2xl font-semibold">{{ t("tokens.title") }}</h1>
      <button v-if="session.role" type="button" class="btn btn-primary" data-test="new-token" @click="creating = true">{{ t("tokens.new") }}</button>
    </div>
    <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("tokens.intro") }}</p>
    <p v-if="error" role="alert" class="alert-error" data-test="tokens-error">{{ error }}</p>
    <p v-if="!store.loaded && !error" class="text-sm text-gray-500">{{ t("common.loading") }}</p>
    <p v-else-if="store.loaded && store.items.length === 0" class="text-sm text-gray-500" data-test="tokens-empty">{{ t("tokens.empty") }}</p>
    <div v-else-if="store.loaded" class="card overflow-x-auto !p-0">
      <table class="table">
        <thead>
          <tr>
            <th>{{ t("tokens.columns.name") }}</th>
            <th>{{ t("tokens.columns.prefix") }}</th>
            <th>{{ t("tokens.columns.role") }}</th>
            <th v-if="showUser" data-test="user-column">{{ t("tokens.columns.user") }}</th>
            <th>{{ t("tokens.columns.created") }}</th>
            <th>{{ t("tokens.columns.expires") }}</th>
            <th>{{ t("tokens.columns.lastUsed") }}</th>
            <th><span class="sr-only">{{ t("common.actions") }}</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="token in store.items" :key="token.id" data-test="token-row">
            <td class="font-medium">{{ token.name }}</td>
            <td class="font-mono text-xs">inv_{{ token.prefix }}_…</td>
            <td>{{ t(`spaces.roles.${token.role}`) }}</td>
            <td v-if="showUser" data-test="token-user">
              <template v-if="token.user">{{ token.user.displayName }} <span class="text-xs text-gray-500">{{ token.user.email }}</span></template>
            </td>
            <td class="whitespace-nowrap">{{ formatDate(token.createdAt, locale) }}</td>
            <td class="whitespace-nowrap">{{ token.expiresAt ? formatDate(token.expiresAt, locale) : t("tokens.never") }}</td>
            <td class="whitespace-nowrap">{{ token.lastUsedAt ? formatDate(token.lastUsedAt, locale) : t("tokens.notUsed") }}</td>
            <td class="text-right">
              <button type="button" class="btn btn-sm btn-danger" data-test="token-revoke" @click="revoke(token)">{{ t("tokens.revoke") }}</button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <McpConnectCard />
    <CreateTokenDialog v-if="creating && session.role" :role="session.role" @close="creating = false" />
  </section>
</template>
