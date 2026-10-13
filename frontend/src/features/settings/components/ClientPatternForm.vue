<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { collectErrors, nullIfEmpty } from "@/lib/formErrors";
import { formatNumber, MAX_PATTERN_LENGTH } from "../numberPattern";
import { useNumberSeriesStore } from "../stores";
import type { NumberSeries } from "../types";

/** The pattern of customers with a number code in one issued series; empty = they use the global one. */
const props = defineProps<{ series: NumberSeries; currentYear: number }>();

/** Code the preview renders `{C}` with. */
const EXAMPLE_CODE = "ACME";
const EXAMPLE_PATTERN = "{C}-{YYYY}{NNN}";

const { t } = useI18n();
const store = useNumberSeriesStore();
const { fieldErrors, error, submitting, submit } = useFormSubmit();

const value = ref(props.series.clientPattern ?? "");
watch(
  () => props.series.clientPattern,
  (saved) => (value.value = saved ?? ""),
);

const trimmed = computed(() => value.value.trim());
const preview = computed(() => (trimmed.value ? formatNumber(trimmed.value, props.currentYear, 1, EXAMPLE_CODE) : null));
const invalid = computed(() => trimmed.value !== "" && preview.value === null);
const dirty = computed(() => nullIfEmpty(value.value) !== props.series.clientPattern);
const inputId = computed(() => `client-pattern-${props.series.docType}`);
const hint = computed(() =>
  t("numbering.clientPattern.hint", {
    example: EXAMPLE_PATTERN,
    result: formatNumber(EXAMPLE_PATTERN, props.currentYear, 1, EXAMPLE_CODE) ?? "",
  }),
);

async function onSubmit() {
  await submit(
    () => collectErrors({ clientPattern: invalid.value && "invalid_pattern" }),
    () => store.saveClientPattern(props.series.docType, nullIfEmpty(value.value)),
  );
}
</script>

<template>
  <form class="space-y-2" novalidate data-test="client-pattern-form" @submit.prevent="onSubmit">
    <FormField :label="t('numbering.clientPattern.label')" :for="inputId" :error="fieldErrors.clientPattern" :hint="hint">
      <div class="flex gap-2">
        <input
          :id="inputId"
          v-model="value"
          :maxlength="MAX_PATTERN_LENGTH"
          class="input font-mono"
          :class="{ 'input-error': fieldErrors.clientPattern || invalid }"
          :placeholder="EXAMPLE_PATTERN"
          autocomplete="off"
          spellcheck="false"
        />
        <button type="submit" class="btn btn-primary shrink-0" :disabled="submitting || !dirty">
          {{ t("common.save") }}
        </button>
      </div>
    </FormField>
    <p v-if="trimmed" class="text-sm" data-test="client-pattern-preview">
      <span class="text-gray-600 dark:text-gray-400">{{ t("numbering.clientPattern.preview", { code: EXAMPLE_CODE }) }}</span>
      <span v-if="preview !== null" class="ml-2 font-mono" data-test="client-pattern-value">{{ preview }}</span>
      <span v-else class="ml-2 text-red-600 dark:text-red-400">{{ t("validation.invalid_pattern") }}</span>
    </p>
    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
  </form>
</template>
