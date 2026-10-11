<script setup lang="ts">
import { onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import { useSessionStore } from "@/stores/session";
import { useToastStore } from "@/stores/toast";
import NewSpaceForm from "../components/NewSpaceForm.vue";
import { useSpacesStore } from "../store";
import type { CreateSpaceBody } from "../types";

const { t } = useI18n();
const session = useSessionStore();
const store = useSpacesStore();
const toast = useToastStore();
const { error, run } = useAction();

onMounted(() => void run(store.load));

async function create(body: CreateSpaceBody): Promise<void> {
  const space = await store.create(body);
  toast.show(t("spaces.create.created", { name: space.name }));
}
</script>

<template>
  <section class="space-y-4">
    <h1 class="text-2xl font-semibold">{{ t("spaces.title") }}</h1>
    <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("spaces.intro") }}</p>

    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    <p v-else-if="!store.loaded" class="text-sm text-gray-500">{{ t("common.loading") }}</p>
    <p v-else-if="store.items.length === 0" class="text-sm text-gray-500" data-test="spaces-empty">{{ t("spaces.empty") }}</p>
    <ul v-else class="card divide-y divide-gray-100 !p-0 dark:divide-gray-800">
      <li v-for="space in store.items" :key="space.slug" class="flex flex-wrap items-center gap-3 px-4 py-3" data-test="space-row">
        <div class="min-w-0 flex-1">
          <p class="font-medium">{{ space.name }}</p>
          <p class="truncate font-mono text-xs text-gray-500">{{ space.url }}</p>
        </div>
        <span class="badge">{{ t(`spaces.roles.${space.role}`) }}</span>
        <!-- Another host: a full page load, and it asks for its own sign-in. -->
        <a :href="space.url" class="btn btn-sm" data-test="space-open">{{ t("spaces.open") }}</a>
      </li>
    </ul>

    <!-- Registration off: only invited people enter, so nobody founds a space (spaces.md). -->
    <p v-if="!session.context?.registration" class="text-sm text-gray-600 dark:text-gray-400" data-test="creation-disabled">
      {{ t("spaces.creationDisabled") }}
    </p>
    <div v-else-if="session.me && !session.me.user.emailVerified" class="card space-y-2" data-test="unverified">
      <p class="text-sm">{{ t("spaces.unverified") }}</p>
      <RouterLink :to="{ name: 'verify' }" class="btn btn-sm">{{ t("spaces.unverifiedAction") }}</RouterLink>
    </div>
    <NewSpaceForm v-else :base-url="session.baseUrl" :create="create" />
  </section>
</template>
