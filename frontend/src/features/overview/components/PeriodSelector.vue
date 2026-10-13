<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { periodErrors } from "@/features/csvExport/period";
import { formatDate, formatMonth } from "@/lib/format";
import { PERIOD_KINDS, quarterOf, shift, switchKind, type Period } from "../period";

const props = defineProps<{ modelValue: Period }>();
const emit = defineEmits<{ "update:modelValue": [Period] }>();
const { t, locale } = useI18n();

const from = ref(props.modelValue.from);
const to = ref(props.modelValue.to);
watch(
  () => props.modelValue,
  (p) => {
    from.value = p.from;
    to.value = p.to;
  },
);
const errors = computed(() => periodErrors(from.value, to.value));

const label = computed(() => {
  const p = props.modelValue;
  const year = p.from.slice(0, 4);
  switch (p.kind) {
    case "month":
      return formatMonth(p.from.slice(0, 7), locale.value);
    case "quarter":
      return t("overview.period.quarterLabel", { quarter: quarterOf(p), year });
    case "year":
      return year;
    default:
      return `${formatDate(p.from, locale.value)} – ${formatDate(p.to, locale.value)}`;
  }
});

const set = (p: Period) => emit("update:modelValue", p);
function apply() {
  if (Object.keys(errors.value).length === 0) set({ kind: "custom", from: from.value, to: to.value });
}
</script>

<template>
  <div class="flex flex-wrap items-center gap-3" data-test="period-selector">
    <div class="inline-flex overflow-hidden rounded-md border border-gray-300 dark:border-gray-700" role="group" :aria-label="t('overview.period.label')">
      <button
        v-for="kind in PERIOD_KINDS"
        :key="kind"
        type="button"
        class="px-3 py-1.5 text-sm"
        :class="
          kind === modelValue.kind
            ? 'bg-gray-900 text-white dark:bg-gray-100 dark:text-gray-900'
            : 'bg-white text-gray-700 hover:bg-gray-50 dark:bg-gray-900 dark:text-gray-300 dark:hover:bg-gray-800'
        "
        :aria-pressed="kind === modelValue.kind"
        :data-test="`period-${kind}`"
        @click="set(switchKind(modelValue, kind))"
      >
        {{ t(`overview.period.kinds.${kind}`) }}
      </button>
    </div>

    <div v-if="modelValue.kind !== 'custom'" class="flex items-center gap-2">
      <button type="button" class="btn btn-sm" :aria-label="t('overview.period.prev')" data-test="period-prev" @click="set(shift(modelValue, -1))">‹</button>
      <span class="min-w-32 text-center font-medium" data-test="period-label">{{ label }}</span>
      <button type="button" class="btn btn-sm" :aria-label="t('overview.period.next')" data-test="period-next" @click="set(shift(modelValue, 1))">›</button>
    </div>

    <form v-else class="flex flex-wrap items-end gap-2" data-test="period-custom-form" @submit.prevent="apply">
      <label class="text-sm">
        <span class="block text-gray-600 dark:text-gray-400">{{ t("overview.period.from") }}</span>
        <input v-model="from" type="date" class="input" data-test="period-from" />
      </label>
      <label class="text-sm">
        <span class="block text-gray-600 dark:text-gray-400">{{ t("overview.period.to") }}</span>
        <input v-model="to" type="date" class="input" :aria-invalid="!!errors.to" data-test="period-to" />
      </label>
      <button type="submit" class="btn" :disabled="Object.keys(errors).length > 0" data-test="period-apply">{{ t("overview.period.apply") }}</button>
    </form>
  </div>
</template>
