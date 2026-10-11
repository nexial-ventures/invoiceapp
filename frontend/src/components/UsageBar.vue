<script setup lang="ts">
import { computed } from "vue";

/** A labelled used / limit bar (storage quotas). `testId` prefixes the `data-test` hooks: `-text`, `-bar`. */
const props = defineProps<{ used: number; limit: number; label: string; hint?: string; testId: string }>();

const percent = computed(() => (props.limit > 0 ? Math.min(100, Math.round((props.used / props.limit) * 100)) : 100));
// Amber from 80 %: a few more uploads can then already hit the limit.
const tone = computed(() => (percent.value >= 95 ? "bg-red-600" : percent.value >= 80 ? "bg-amber-500" : "bg-blue-600"));
</script>

<template>
  <div class="space-y-1" :data-test="testId">
    <div class="flex flex-wrap justify-between gap-2 text-sm">
      <span :data-test="`${testId}-text`">{{ label }}</span>
      <span v-if="hint" class="text-gray-500">{{ hint }}</span>
    </div>
    <div
      class="h-2 overflow-hidden rounded-full bg-gray-200 dark:bg-gray-800"
      role="progressbar"
      :aria-valuenow="percent"
      aria-valuemin="0"
      aria-valuemax="100"
      :aria-label="label"
    >
      <div class="h-full" :class="tone" :style="{ width: `${percent}%` }" :data-test="`${testId}-bar`" />
    </div>
  </div>
</template>
