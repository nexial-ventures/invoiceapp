<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { formatDate, formatNumber } from "@/lib/format";
import type { Document } from "@/features/documents/types";

const props = defineProps<{ doc: Document }>();
const { t, locale } = useI18n();

const rows = computed(() => {
  const d = props.doc;
  const date = (v: string | null | undefined) => formatDate(v, locale.value);
  const all: [string, string | null | undefined][] = [
    ["received.fields.supplierNumber", d.supplierNumber],
    ["documents.fields.issueDate", date(d.issueDate)],
    ["documents.fields.taxPointDate", date(d.taxPointDate)],
    ["received.fields.receivedDate", date(d.receivedDate)],
    ["documents.fields.dueDate", date(d.dueDate)],
    ["documents.fields.currency", d.currency],
    [
      "documents.fields.exchangeRate",
      d.currency === "CZK" || !d.exchangeRate
        ? null
        : [
            formatNumber(d.exchangeRate, locale.value, 4),
            d.exchangeRateSource && t(`documents.rateSource.${d.exchangeRateSource}`),
            d.exchangeRateDate && date(d.exchangeRateDate),
          ]
            .filter(Boolean)
            .join(" · "),
    ],
    ["documents.fields.vatMode", d.vatMode === "non_payer" ? t("received.supplierNonPayer") : t(`documents.vatMode.${d.vatMode}`)],
    ["documents.fields.roundingMode", t(`documents.roundingMode.${d.roundingMode}`)],
    ["received.fields.vatDeductible", d.vatDeductible == null ? null : d.vatDeductible ? t("common.yes") : t("common.no")],
    ["documents.fields.variableSymbol", d.variableSymbol],
    ["documents.fields.constantSymbol", d.constantSymbol],
    ["received.fields.supplierAccount", d.supplierAccount],
  ];
  return all.filter((r): r is [string, string] => !!r[1]);
});
</script>

<template>
  <section class="card">
    <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm sm:grid-cols-[auto_1fr_auto_1fr]">
      <template v-for="[label, value] in rows" :key="label">
        <dt class="text-gray-600 dark:text-gray-400">{{ t(label) }}</dt>
        <dd :data-test="`info-${label.split('.').pop()}`">{{ value }}</dd>
      </template>
    </dl>
  </section>
</template>
