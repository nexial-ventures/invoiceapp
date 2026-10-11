<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { formatDate, formatNumber } from "../format";
import type { Document } from "../types";

const props = defineProps<{ doc: Document }>();
const { t, locale } = useI18n();

const bank = computed(() => {
  const b = props.doc.bankSnapshot;
  return b ? [b.accountNumber, b.iban, b.bic].filter(Boolean).join(" · ") : null;
});

const rows = computed(() => {
  const d = props.doc;
  const date = (v: string | null) => formatDate(v, locale.value);
  const all: [string, string | null][] = [
    ["issueDate", date(d.issueDate)],
    ["taxPointDate", date(d.taxPointDate)],
    ["dueDate", date(d.dueDate)],
    ["currency", d.currency],
    [
      "exchangeRate",
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
    ["vatMode", t(`documents.vatMode.${d.vatMode}`)],
    ["paymentMethod", t(`documents.paymentMethod.${d.paymentMethod}`)],
    ["roundingMode", t(`documents.roundingMode.${d.roundingMode}`)],
    ["bankAccountId", bank.value],
    ["variableSymbol", d.variableSymbol],
    ["constantSymbol", d.constantSymbol],
    ["orderRef", d.orderRef],
    ["locale", t(`locale.names.${d.locale}`)],
    ["sentAt", d.sentAt && date(d.sentAt)],
    ["cancelledAt", d.cancelledAt && date(d.cancelledAt)],
    ["cancelReason", d.cancelReason],
    ["correctionReason", d.correctionReason],
  ];
  return all.filter((r): r is [string, string] => !!r[1]);
});
</script>

<template>
  <section class="card space-y-4">
    <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm sm:grid-cols-[auto_1fr_auto_1fr]">
      <template v-for="[field, value] in rows" :key="field">
        <dt class="text-gray-600 dark:text-gray-400">{{ t(`documents.fields.${field}`) }}</dt>
        <dd :data-test="`info-${field}`">{{ value }}</dd>
      </template>
    </dl>
    <div v-if="doc.headerNote || doc.footerNote" class="space-y-2 border-t border-gray-200 pt-3 text-sm dark:border-gray-800">
      <p v-if="doc.headerNote" class="whitespace-pre-line">{{ doc.headerNote }}</p>
      <p v-if="doc.footerNote" class="whitespace-pre-line text-gray-600 dark:text-gray-400">{{ doc.footerNote }}</p>
    </div>
  </section>
</template>
