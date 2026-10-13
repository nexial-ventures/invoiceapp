<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { formatMoney, formatNumber } from "@/lib/format";
import { isNegative, parseCents } from "@/lib/money";
import type { OverviewVat } from "../types";

const props = defineProps<{ vat: OverviewVat }>();
const { t, locale } = useI18n();
const money = (v: string) => formatMoney(v, "CZK", locale.value);
const nonZero = (v: string) => (parseCents(v) ?? 0n) !== 0n;

const sections = computed(() => [
  { key: "output", rows: props.vat.output, total: props.vat.outputVat },
  { key: "input", rows: props.vat.input, total: props.vat.inputVat },
]);
const refund = computed(() => isNegative(props.vat.result));
const result = computed(() => (refund.value ? props.vat.result.replace(/^-/, "") : props.vat.result));
</script>

<template>
  <div class="card overflow-x-auto !p-0">
    <table class="table" data-test="vat-table">
      <thead>
        <tr>
          <th>{{ t("overview.vat.rate") }}</th>
          <th class="text-right">{{ t("overview.vat.base") }}</th>
          <th class="text-right">{{ t("overview.vat.vat") }}</th>
        </tr>
      </thead>
      <tbody v-for="s in sections" :key="s.key" :data-test="`vat-${s.key}`">
        <tr>
          <th colspan="3" class="bg-gray-50 text-left dark:bg-gray-800/60">{{ t(`overview.vat.${s.key}`) }}</th>
        </tr>
        <tr v-for="r in s.rows" :key="r.rate" data-test="vat-row">
          <td>{{ formatNumber(r.rate, locale) }} %</td>
          <td class="text-right tabular-nums">{{ money(r.base) }}</td>
          <td class="text-right tabular-nums">{{ money(r.vat) }}</td>
        </tr>
        <tr v-if="s.rows.length === 0">
          <td colspan="3" class="text-gray-500">{{ t("overview.vat.none") }}</td>
        </tr>
        <tr class="font-medium">
          <td colspan="2">{{ t(`overview.vat.${s.key}Total`) }}</td>
          <td class="text-right tabular-nums">{{ money(s.total) }}</td>
        </tr>
      </tbody>
      <tbody>
        <tr class="font-semibold" data-test="vat-result">
          <td colspan="2">{{ t(refund ? "overview.vat.refund" : "overview.vat.toPay") }}</td>
          <td class="text-right tabular-nums">{{ money(result) }}</td>
        </tr>
        <tr v-if="nonZero(vat.reverseChargeIssued)" data-test="vat-rc-issued">
          <td>{{ t("overview.vat.reverseChargeIssued") }}</td>
          <td class="text-right tabular-nums">{{ money(vat.reverseChargeIssued) }}</td>
          <td class="text-right text-gray-500">—</td>
        </tr>
        <tr v-if="nonZero(vat.reverseChargeReceived)" data-test="vat-rc-received">
          <td>{{ t("overview.vat.reverseChargeReceived") }}</td>
          <td class="text-right tabular-nums">{{ money(vat.reverseChargeReceived) }}</td>
          <td class="text-right text-gray-500">—</td>
        </tr>
        <tr v-if="nonZero(vat.nonDeductible.base) || nonZero(vat.nonDeductible.vat)" class="text-gray-600 dark:text-gray-400" data-test="vat-non-deductible">
          <td>{{ t("overview.vat.nonDeductible") }}</td>
          <td class="text-right tabular-nums">{{ money(vat.nonDeductible.base) }}</td>
          <td class="text-right tabular-nums">{{ money(vat.nonDeductible.vat) }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
