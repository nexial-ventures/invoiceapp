<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { formatMoney } from "@/lib/format";
import { PREVIEW_CLASS } from "@/features/imports/badges";
import ImportCounterparty from "@/features/imports/components/ImportCounterparty.vue";
import ImportDates from "@/features/imports/components/ImportDates.vue";
import { useIsdocImportStore } from "../store";
import type { PreviewEntry } from "../types";
import { entryErrorKey, warningKey } from "../upload";

/** Preview rows with their selection checkboxes; only `ok` rows are selectable. */
const { t, locale } = useI18n();
const store = useIsdocImportStore();

const entries = computed(() => store.entries ?? []);
const checked = (event: Event) => (event.target as HTMLInputElement).checked;

const typeLabel = (e: PreviewEntry) =>
  e.docType ? t(`${e.direction === "received" ? "received" : "documents"}.docTypes.${e.docType}`) : "—";
const messageParams = (e: PreviewEntry, code: string) => ({ code, number: e.relatedNumber ?? "" });
</script>

<template>
  <div class="card overflow-x-auto !p-0">
    <table class="table" data-test="isdoc-preview">
      <thead>
        <tr>
          <th class="w-8">
            <input
              type="checkbox"
              :checked="store.allSelected"
              :disabled="store.okKeys.length === 0"
              :aria-label="t('imports.preview.selectAll')"
              data-test="isdoc-select-all"
              @change="store.toggleAll(checked($event))"
            />
          </th>
          <th>{{ t("imports.preview.status") }}</th>
          <th>{{ t("imports.preview.document") }}</th>
          <th>{{ t("imports.preview.counterparty") }}</th>
          <th class="hidden md:table-cell">{{ t("imports.preview.dates") }}</th>
          <th class="text-right">{{ t("imports.preview.total") }}</th>
          <th class="hidden sm:table-cell">PDF</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="e in entries" :key="e.key" data-test="isdoc-preview-row">
          <td class="align-top">
            <input
              type="checkbox"
              :checked="store.isSelected(e)"
              :disabled="e.status !== 'ok'"
              :aria-label="t('imports.preview.select', { key: e.key })"
              data-test="isdoc-select"
              @change="store.toggle(e.key, checked($event))"
            />
          </td>
          <td class="align-top">
            <span class="badge" :class="PREVIEW_CLASS[e.status]" data-test="isdoc-status">{{ t(`imports.status.${e.status}`) }}</span>
          </td>
          <td class="align-top">
            <div class="font-mono font-medium">{{ e.number ?? "—" }}</div>
            <div class="text-xs text-gray-600 dark:text-gray-400">
              <span v-if="e.direction">{{ t(`imports.direction.${e.direction}`) }} · </span>{{ typeLabel(e) }}
            </div>
            <div class="text-xs break-all text-gray-500" data-test="isdoc-key">{{ e.key }}</div>
            <p v-if="e.error" class="mt-1 text-xs text-red-700 dark:text-red-400" data-test="isdoc-error">
              {{ t(entryErrorKey(e.error), messageParams(e, e.error)) }}
            </p>
            <ul v-if="e.warnings.length" class="mt-1 list-disc pl-4 text-xs text-amber-700 dark:text-amber-400" data-test="isdoc-warnings">
              <li v-for="w in e.warnings" :key="w">{{ t(warningKey(w), messageParams(e, w)) }}</li>
            </ul>
          </td>
          <td class="align-top">
            <ImportCounterparty :entry="e" />
          </td>
          <td class="hidden align-top text-xs whitespace-nowrap md:table-cell">
            <ImportDates :entry="e" />
          </td>
          <td class="text-right align-top whitespace-nowrap tabular-nums" data-test="isdoc-total">
            {{ e.total && e.currency ? formatMoney(e.total, e.currency, locale) : "—" }}
          </td>
          <td class="hidden align-top sm:table-cell" data-test="isdoc-pdf">{{ e.hasPdf ? t("common.yes") : t("common.no") }}</td>
        </tr>
        <tr v-if="entries.length === 0">
          <td colspan="7" class="py-8 text-center text-gray-500">{{ t("isdoc.preview.empty") }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
