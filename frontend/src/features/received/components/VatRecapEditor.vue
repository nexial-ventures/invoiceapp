<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { FieldErrors } from "@/api/types";
import FormField from "@/components/form/FormField.vue";
import { reasonKey } from "@/lib/formErrors";
import { docSign } from "@/features/documents/docTypes";
import { formatMoney } from "@/lib/format";
import { draftTotal, type ReceivedDraft } from "../form";
import { derivedRoundingMode, newRecapRow, updateRow, vatAllowed, type RecapRowDraft } from "../recap";

/** VAT recap rows (rate / base / VAT), rounding, the computed total and the payable amount. */
const model = defineModel<ReceivedDraft>({ required: true });
const props = defineProps<{
  errors: FieldErrors;
  /** Rates from Settings, offered as suggestions; any 0–100 rate is accepted (foreign VAT, old rates). */
  rateOptions: string[];
  defaultRate: string;
}>();

const { t, locale } = useI18n();
const vatEnabled = computed(() => vatAllowed(model.value.vatMode));
const total = computed(() => draftTotal(model.value));
const roundingMode = computed(() => derivedRoundingMode(model.value.vatRecap, model.value.rounding));
const isCreditNote = computed(() => docSign(model.value.docType) === -1);
const rowError = (i: number, field: string) => props.errors[`vatRecap.${i}.${field}`];

function edit(i: number, patch: Partial<Pick<RecapRowDraft, "rate" | "base" | "vat">>) {
  model.value.vatRecap = model.value.vatRecap.map((r, j) => (j === i ? updateRow(r, patch, model.value.vatMode) : r));
}
function add() {
  const used = new Set(model.value.vatRecap.map((r) => r.rate));
  const rate = [props.defaultRate, ...props.rateOptions].find((r) => !used.has(r)) ?? "";
  model.value.vatRecap = [...model.value.vatRecap, updateRow(newRecapRow(vatEnabled.value ? rate : "0"), {}, model.value.vatMode)];
}
function remove(i: number) {
  model.value.vatRecap = model.value.vatRecap.filter((_, j) => j !== i);
}
function onPayable(event: Event) {
  const value = (event.target as HTMLInputElement).value;
  model.value = { ...model.value, payable: value, payableAuto: value.trim() === "" };
}
const value = (e: Event) => (e.target as HTMLInputElement).value;
</script>

<template>
  <div class="space-y-4">
    <div class="flex items-baseline justify-between gap-2">
      <h2 class="font-semibold">{{ t("received.recap.title") }}</h2>
      <span v-if="isCreditNote" class="text-xs text-gray-600 dark:text-gray-400">{{ t("received.recap.creditNoteHint") }}</span>
    </div>
    <p v-if="errors.vatRecap" class="text-xs text-red-600 dark:text-red-400" data-test="recap-error">{{ t(reasonKey(errors.vatRecap)) }}</p>
    <datalist id="recap-rate-options">
      <option v-for="r in rateOptions" :key="r" :value="r" />
    </datalist>
    <div class="overflow-x-auto">
      <table class="table">
        <thead>
          <tr>
            <th>{{ t("received.recap.rate") }}</th>
            <th>{{ t("documents.totals.base") }}</th>
            <th>{{ t("documents.totals.vat") }}</th>
            <th class="w-0"><span class="sr-only">{{ t("common.actions") }}</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, i) in model.vatRecap" :key="row.key" data-test="recap-edit-row">
            <td class="min-w-24 align-top">
              <input
                :id="`recap-${i}-rate`"
                :value="row.rate"
                list="recap-rate-options"
                inputmode="decimal"
                class="input"
                :class="{ 'input-error': rowError(i, 'rate') }"
                :aria-label="t('received.recap.rateOf', { n: i + 1 })"
                autocomplete="off"
                @input="edit(i, { rate: value($event) })"
              />
              <p v-if="rowError(i, 'rate')" class="text-xs text-red-600 dark:text-red-400" data-test="field-error">{{ t(reasonKey(rowError(i, "rate") ?? "")) }}</p>
            </td>
            <td class="min-w-32 align-top">
              <input
                :id="`recap-${i}-base`"
                :value="row.base"
                inputmode="decimal"
                class="input"
                :class="{ 'input-error': rowError(i, 'base') }"
                :aria-label="t('received.recap.baseOf', { n: i + 1 })"
                autocomplete="off"
                @input="edit(i, { base: value($event) })"
              />
              <p v-if="rowError(i, 'base')" class="text-xs text-red-600 dark:text-red-400" data-test="field-error">{{ t(reasonKey(rowError(i, "base") ?? "")) }}</p>
            </td>
            <td class="min-w-32 align-top">
              <input
                :id="`recap-${i}-vat`"
                :value="row.vat"
                inputmode="decimal"
                class="input"
                :class="{ 'input-error': rowError(i, 'vat') }"
                :disabled="!vatEnabled"
                :aria-label="t('received.recap.vatOf', { n: i + 1 })"
                autocomplete="off"
                @input="edit(i, { vat: value($event) })"
              />
              <p v-if="rowError(i, 'vat')" class="text-xs text-red-600 dark:text-red-400" data-test="field-error">{{ t(reasonKey(rowError(i, "vat") ?? "")) }}</p>
              <p v-else-if="vatEnabled && row.vatAuto && row.vat" class="text-xs text-gray-500">{{ t("received.recap.vatSuggested") }}</p>
            </td>
            <td class="align-top">
              <button type="button" class="btn btn-sm" :disabled="model.vatRecap.length === 1" :aria-label="t('received.recap.remove', { n: i + 1 })" data-test="remove-recap-row" @click="remove(i)">✕</button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <button type="button" class="btn btn-sm" data-test="add-recap-row" @click="add">+ {{ t("received.recap.addRow") }}</button>
    <p v-if="!vatEnabled" class="text-xs text-gray-600 dark:text-gray-400">{{ t("received.recap.noVat") }}</p>

    <div class="grid gap-4 border-t border-gray-200 pt-4 sm:grid-cols-3 dark:border-gray-800">
      <FormField
        :label="t('documents.totals.rounding')"
        for="rec-rounding"
        :error="errors.rounding"
        :hint="roundingMode ? t('received.recap.roundingMode', { mode: t(`documents.roundingMode.${roundingMode}`) }) : undefined"
      >
        <input id="rec-rounding" v-model="model.rounding" inputmode="decimal" class="input" :class="{ 'input-error': errors.rounding }" autocomplete="off" />
      </FormField>
      <div class="space-y-1">
        <span class="block text-sm font-medium">{{ t("documents.totals.total") }}</span>
        <p class="py-2 text-sm font-semibold tabular-nums" data-test="recap-total">{{ total === null ? "—" : formatMoney(total, model.currency, locale) }}</p>
      </div>
      <FormField :label="t('documents.totals.payable')" for="rec-payable" :error="errors.payable" :hint="model.payableAuto ? t('received.recap.payableAuto') : t('received.recap.payableManual')">
        <input id="rec-payable" :value="model.payable" inputmode="decimal" class="input" :class="{ 'input-error': errors.payable }" autocomplete="off" @input="onPayable" />
      </FormField>
    </div>
  </div>
</template>
