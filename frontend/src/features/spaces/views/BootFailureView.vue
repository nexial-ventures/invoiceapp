<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useSessionStore } from "@/stores/session";

const { t, te } = useI18n();
// Not even the messages loaded (e.g. a stale cached page after a redeploy): plain text then.
const hasMessages = te("spaces.bootError.title");
const session = useSessionStore();

function retry() {
  window.location.reload();
}
</script>

<template>
  <div class="flex min-h-[70dvh] items-center justify-center">
    <div v-if="session.bootFailure?.status === 'not_found'" class="card w-full max-w-md space-y-3" data-test="space-not-found">
      <h1 class="text-xl font-semibold">{{ t("spaces.notFound.title") }}</h1>
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("spaces.notFound.text") }}</p>
      <a :href="session.bootFailure.baseUrl" class="btn btn-primary" data-test="base-link">{{ t("spaces.notFound.link") }}</a>
    </div>
    <div v-else class="card w-full max-w-md space-y-3" data-test="boot-error">
      <template v-if="hasMessages">
        <h1 class="text-xl font-semibold">{{ t("spaces.bootError.title") }}</h1>
        <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("spaces.bootError.text") }}</p>
        <button type="button" class="btn btn-primary" @click="retry">{{ t("spaces.bootError.retry") }}</button>
      </template>
      <template v-else>
        <h1 class="text-xl font-semibold">Aplikaci se nepodařilo načíst / The app failed to load</h1>
        <button type="button" class="btn btn-primary" data-test="boot-reload" @click="retry">Načíst znovu / Reload</button>
      </template>
    </div>
  </div>
</template>
