<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import UsageBar from "@/components/UsageBar.vue";
import { formatBytes } from "@/lib/bytes";
import type { SpaceLimits } from "../types";

/** The space's storage quota and what it uses (read-only: the operator changes the quota). */
const props = defineProps<{ limits: SpaceLimits }>();
const { t, locale } = useI18n();

const label = computed(() =>
  t("spaces.storage.usage", {
    used: formatBytes(props.limits.storageUsedBytes, locale.value),
    limit: formatBytes(props.limits.storageQuotaBytes, locale.value),
  }),
);
</script>

<template>
  <section class="card space-y-3" data-test="storage-usage">
    <h2 class="text-lg font-semibold">{{ t("spaces.storage.title") }}</h2>
    <UsageBar :used="limits.storageUsedBytes" :limit="limits.storageQuotaBytes" :label="label" test-id="space-storage" />
    <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("spaces.storage.hint") }}</p>
  </section>
</template>
