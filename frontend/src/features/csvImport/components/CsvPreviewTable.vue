<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { formatMoney } from "@/lib/format";
import { MATCH_CLASS, PREVIEW_CLASS } from "@/features/imports/badges";
import ImportCounterparty from "@/features/imports/components/ImportCounterparty.vue";
import ImportDates from "@/features/imports/components/ImportDates.vue";
import { rowErrorKey, warningKey } from "../codes";
import { useCsvImportStore } from "../store";
import type { CsvPreviewEntry } from "../types";

/** One preview row per file row; only `ok` rows are selectable. */
const { t, locale } = useI18n();
const store = useCsvImportStore();

const entries = computed(() => store.entries ?? []);
const checked = (event: Event) => (event.target as HTMLInputElement).checked;

const typeLabel = (e: CsvPreviewEntry) =>
  e.docType ? t(`${e.direction === "received" ? "received" : "documents"}.docTypes.${e.docType}`) : "—";
const messageParams = (e: CsvPreviewEntry, code: string) => ({ code, number: e.relatedNumber ?? "" });
</script>

<template>
  <div class="card overflow-x-auto !p-0">
    <table class="table" data-test="csv-preview">
      <thead>
        <tr>
          <th class="w-8">
            <input
              type="checkbox"
              :checked="store.allSelected"
              :disabled="store.okKeys.length === 0"
              :aria-label="t('imports.preview.selectAll')"
              data-test="csv-select-all"
              @change="store.toggleAll(checked($event))"
            />
          </th>
          <th class="text-right">{{ t("csvImport.preview.row") }}</th>
          <th>{{ t("imports.preview.status") }}</th>
          <th>{{ t("imports.preview.document") }}</th>
          <th>{{ t("imports.preview.counterparty") }}</th>
          <th class="hidden md:table-cell">{{ t("imports.preview.dates") }}</th>
          <th class="text-right">{{ t("imports.preview.total") }}</th>
          <th class="hidden sm:table-cell">{{ t("csvImport.preview.category") }}</th>
          <th>{{ t("csvImport.preview.messages") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="e in entries" :key="e.key" data-test="csv-preview-row">
          <td class="align-top">
            <input
              type="checkbox"
              :checked="store.isSelected(e)"
              :disabled="e.status !== 'ok'"
              :aria-label="t('csvImport.preview.select', { n: e.row })"
              data-test="csv-select"
              @change="store.toggle(e.key, checked($event))"
            />
          </td>
          <td class="text-right align-top tabular-nums" data-test="csv-row">{{ e.row }}</td>
          <td class="align-top">
            <span class="badge" :class="PREVIEW_CLASS[e.status]" data-test="csv-status">{{ t(`imports.status.${e.status}`) }}</span>
          </td>
          <td class="align-top">
            <div class="font-mono font-medium" data-test="csv-number">{{ e.number ?? "—" }}</div>
            <div class="text-xs text-gray-600 dark:text-gray-400" data-test="csv-type">
              <span v-if="e.direction">{{ t(`imports.direction.${e.direction}`) }} · </span>{{ typeLabel(e) }}
            </div>
          </td>
          <td class="align-top">
            <ImportCounterparty :entry="e" />
          </td>
          <td class="hidden align-top text-xs whitespace-nowrap md:table-cell">
            <ImportDates :entry="e" />
          </td>
          <td class="text-right align-top whitespace-nowrap tabular-nums" data-test="csv-total">
            {{ e.total && e.currency ? formatMoney(e.total, e.currency, locale) : "—" }}
          </td>
          <td class="hidden align-top sm:table-cell">
            <span v-if="e.categoryMatch" class="badge" :class="MATCH_CLASS[e.categoryMatch]" data-test="csv-category">
              {{ t(`csvImport.categoryMatch.${e.categoryMatch}`) }}
            </span>
            <template v-else>—</template>
          </td>
          <td class="align-top text-xs">
            <div v-if="e.error" class="text-red-700 dark:text-red-400" data-test="csv-error">
              <p>{{ t(rowErrorKey(e.error, e.field), messageParams(e, e.error)) }}</p>
              <p v-if="e.field" data-test="csv-field">{{ t("csvImport.preview.column", { column: e.field }) }}</p>
            </div>
            <ul v-if="e.warnings.length" class="list-disc pl-4 text-amber-700 dark:text-amber-400" data-test="csv-warnings">
              <li v-for="w in e.warnings" :key="w">{{ t(warningKey(w), messageParams(e, w)) }}</li>
            </ul>
          </td>
        </tr>
        <tr v-if="entries.length === 0">
          <td colspan="9" class="py-8 text-center text-gray-500">{{ t("csvImport.preview.empty") }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
