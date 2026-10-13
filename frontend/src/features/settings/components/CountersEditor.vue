<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useErrorText } from "@/composables/useAction";
import { fieldErrorsOf, reasonKey } from "@/lib/formErrors";
import type { NumberSeries } from "../types";

/**
 * Counters per year of one sequence: the global series (Settings) or a customer's client sequence.
 * `saveCounter` upserts one year; `readonly` lists the values without inputs.
 */
const props = defineProps<{
  counters: NumberSeries["counters"];
  currentYear: number;
  saveCounter: (year: number, lastNumber: number) => Promise<void>;
  readonly?: boolean;
}>();

const { t } = useI18n();
const errorText = useErrorText();
const error = ref<string | null>(null);

const sorted = computed(() => [...props.counters].sort((a, b) => b.year - a.year));
/** Edited lastNumber per year, as typed. */
const drafts = ref<Record<number, string>>({});

function defaultNewYear(): string {
  const taken = props.counters.some((c) => c.year === props.currentYear);
  return String(taken ? props.currentYear + 1 : props.currentYear);
}

const newYear = ref(defaultNewYear());
const newLast = ref("0");

// Only seed drafts for years that appear; existing drafts (possibly unsaved
// edits in other rows) are left alone.
watch(
  () => props.counters,
  (counters) => {
    for (const c of counters) {
      if (!(c.year in drafts.value)) drafts.value[c.year] = String(c.lastNumber);
    }
  },
  { immediate: true },
);

const isCount = (v: string) => /^\d{1,9}$/.test(v.trim());

async function save(yearText: string, value: string, isNewRow = false) {
  if (!/^\d{4}$/.test(yearText.trim()) || !isCount(value)) {
    error.value = t("settings.numberSeries.invalidCounter");
    return;
  }
  const year = Number(yearText);
  const lastNumber = Number(value.trim());
  error.value = null;
  try {
    await props.saveCounter(year, lastNumber);
  } catch (err) {
    // The field reason is the useful part, not "form has errors".
    const fields = fieldErrorsOf(err);
    error.value = fields ? Object.values(fields).map((r) => t(reasonKey(r))).join(" ") : errorText(err);
    return;
  }
  // Reset just the saved row; other rows keep whatever is being typed.
  drafts.value[year] = String(lastNumber);
  if (isNewRow) {
    newYear.value = String(year + 1);
    newLast.value = "0";
  }
}
</script>

<template>
  <div class="space-y-2">
    <h3 class="text-sm font-medium">{{ t("settings.numberSeries.counters") }}</h3>
    <table class="table max-w-md">
      <thead>
        <tr>
          <th>{{ t("settings.numberSeries.year") }}</th>
          <th>{{ t("settings.numberSeries.lastNumber") }}</th>
          <th v-if="!readonly" class="w-0"><span class="sr-only">{{ t("common.actions") }}</span></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="c in sorted" :key="c.year">
          <td class="font-mono">{{ c.year }}</td>
          <td v-if="readonly" class="font-mono" data-test="counter-value">{{ c.lastNumber }}</td>
          <template v-else>
            <td>
              <input
                v-model="drafts[c.year]"
                inputmode="numeric"
                class="input py-1"
                :aria-label="t('settings.numberSeries.lastNumberFor', { year: c.year })"
              />
            </td>
            <td>
              <button
                type="button"
                class="btn btn-sm"
                :disabled="drafts[c.year] === String(c.lastNumber)"
                @click="save(String(c.year), drafts[c.year] ?? '')"
              >
                {{ t("common.save") }}
              </button>
            </td>
          </template>
        </tr>
        <tr v-if="!readonly">
          <td>
            <input v-model="newYear" inputmode="numeric" maxlength="4" class="input py-1" :aria-label="t('settings.numberSeries.year')" />
          </td>
          <td>
            <input v-model="newLast" inputmode="numeric" class="input py-1" :aria-label="t('settings.numberSeries.lastNumber')" />
          </td>
          <td>
            <button type="button" class="btn btn-sm" @click="save(newYear, newLast, true)">{{ t("common.add") }}</button>
          </td>
        </tr>
      </tbody>
    </table>
    <p v-if="!readonly" class="text-xs text-gray-500 dark:text-gray-400" data-test="counter-hint">{{ t("numbering.counterHint") }}</p>
    <p v-if="error" role="alert" class="alert-error" data-test="counter-error">{{ error }}</p>
  </div>
</template>
