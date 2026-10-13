<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { ISSUED_SERIES } from "@/api/types";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { collectErrors } from "@/lib/formErrors";
import { formatNumber, MAX_PATTERN_LENGTH } from "../numberPattern";
import { useNumberSeriesStore } from "../stores";
import type { NumberSeries } from "../types";
import ClientPatternForm from "./ClientPatternForm.vue";
import CountersEditor from "./CountersEditor.vue";

const props = defineProps<{ series: NumberSeries; currentYear: number }>();

const { t } = useI18n();
const store = useNumberSeriesStore();
const { fieldErrors, error, submitting, submit } = useFormSubmit();

const pattern = ref(props.series.pattern);
watch(
  () => props.series.pattern,
  (saved) => (pattern.value = saved),
);

// Validates the pattern being typed; the real next number is the server's `nextNumberPreview`
// (it skips numbers already held, which the counter alone does not tell).
const preview = computed(() => formatNumber(pattern.value.trim(), props.currentYear, 1));
const dirty = computed(() => pattern.value.trim() !== props.series.pattern);
const inputId = computed(() => `pattern-${props.series.docType}`);
/** Client patterns exist for issued series only (received documents always use the global one). */
const issued = computed(() => (ISSUED_SERIES as readonly string[]).includes(props.series.docType));
const saveCounter = (year: number, lastNumber: number) => store.setCounter(props.series.docType, year, lastNumber);

async function onSubmit() {
  const value = pattern.value.trim();
  await submit(
    () => collectErrors({ pattern: preview.value === null && "invalid_pattern" }),
    () => store.savePattern(props.series.docType, value),
  );
}
</script>

<template>
  <section class="card space-y-4" :data-test="`series-${series.docType}`">
    <h2 class="font-semibold">{{ t(`settings.numberSeries.docTypes.${series.docType}`) }}</h2>

    <form class="space-y-2" novalidate @submit.prevent="onSubmit">
      <FormField :label="t('settings.numberSeries.pattern')" :for="inputId" :error="fieldErrors.pattern">
        <div class="flex gap-2">
          <input
            :id="inputId"
            v-model="pattern"
            :maxlength="MAX_PATTERN_LENGTH"
            class="input font-mono"
            :class="{ 'input-error': fieldErrors.pattern || preview === null }"
            autocomplete="off"
            spellcheck="false"
          />
          <button type="submit" class="btn btn-primary shrink-0" :disabled="submitting || !dirty">
            {{ t("common.save") }}
          </button>
        </div>
      </FormField>
      <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
        <dt v-if="dirty || preview === null" class="text-gray-600 dark:text-gray-400">{{ t("settings.numberSeries.preview") }}</dt>
        <dd v-if="dirty || preview === null" class="font-mono" data-test="client-preview">
          <template v-if="preview !== null">{{ preview }}</template>
          <span v-else class="text-red-600 dark:text-red-400">{{ t("validation.invalid_pattern") }}</span>
        </dd>
        <dt class="text-gray-600 dark:text-gray-400">{{ t("settings.numberSeries.serverPreview") }}</dt>
        <dd class="font-mono" data-test="server-preview">{{ series.nextNumberPreview }}</dd>
      </dl>
      <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    </form>

    <ClientPatternForm v-if="issued" :series="series" :current-year="currentYear" />

    <CountersEditor :counters="series.counters" :current-year="currentYear" :save-counter="saveCounter" />
  </section>
</template>
