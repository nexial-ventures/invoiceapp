<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { formatBytes } from "@/lib/bytes";
import type { DesignQuota } from "../../types";

const props = defineProps<{ quota: DesignQuota }>();
const { t, locale } = useI18n();

const percent = computed(() => (props.quota.limit > 0 ? Math.min(100, Math.round((props.quota.used / props.quota.limit) * 100)) : 100));
// Amber from 80 %: an upload of a few fonts can then already hit the limit.
const tone = computed(() => (percent.value >= 95 ? "bg-red-600" : percent.value >= 80 ? "bg-amber-500" : "bg-blue-600"));
</script>

<template>
  <div class="space-y-1" data-test="design-quota">
    <div class="flex flex-wrap justify-between gap-2 text-sm">
      <span data-test="design-quota-text">
        {{ t("design.quota", { used: formatBytes(quota.used, locale), limit: formatBytes(quota.limit, locale) }) }}
      </span>
      <span class="text-gray-500">{{ t("design.quotaHint") }}</span>
    </div>
    <div
      class="h-2 overflow-hidden rounded-full bg-gray-200 dark:bg-gray-800"
      role="progressbar"
      :aria-valuenow="percent"
      aria-valuemin="0"
      aria-valuemax="100"
      :aria-label="t('design.quota', { used: formatBytes(quota.used, locale), limit: formatBytes(quota.limit, locale) })"
    >
      <div class="h-full" :class="tone" :style="{ width: `${percent}%` }" data-test="design-quota-bar" />
    </div>
  </div>
</template>
