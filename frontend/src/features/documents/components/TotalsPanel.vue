<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { isNegative, negate, parseCents, signed } from "@/lib/money";
import { formatMoney, formatNumber } from "@/lib/format";
import type { Sign, Totals } from "../types";

const props = defineProps<{
  totals: Totals | null;
  currency: string;
  exchangeRate?: string | null;
  /** A newer computation is on its way. */
  pending?: boolean;
  /** Translated failure of the last computation; the previous totals stay visible. */
  error?: string | null;
  /** -1 shows a credit note's (stored positive) amounts negated. */
  sign?: Sign;
}>();

const { t, locale } = useI18n();

const foreign = computed(() => props.currency !== "CZK");
const sign = computed<Sign>(() => props.sign ?? 1);
const display = (v: string | null | undefined) => (v == null || v === "" ? v : signed(v, sign.value));
const money = (v: string | null | undefined) => formatMoney(display(v), props.currency, locale.value);
const czk = (v: string | null | undefined) => formatMoney(display(v), "CZK", locale.value);
/** Advances above the invoice total: the customer gets the difference back. */
const overpaid = computed(() => sign.value === 1 && isNegative(props.totals?.payable));
const showCzk = computed(() => foreign.value && props.totals?.recap.some((r) => r.baseCzk !== null));
</script>

<template>
  <section class="card space-y-3" :aria-busy="pending" data-test="totals-panel">
    <div class="flex items-center justify-between">
      <h2 class="font-semibold">{{ t("documents.totals.title") }}</h2>
      <span v-if="pending" class="text-xs text-gray-500">{{ t("documents.totals.computing") }}</span>
    </div>
    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>

    <template v-if="totals">
      <div class="overflow-x-auto" :class="{ 'opacity-60': pending }">
        <table class="table">
          <thead>
            <tr>
              <th>{{ t("documents.totals.rate") }}</th>
              <th class="text-right">{{ t("documents.totals.base") }}</th>
              <th class="text-right">{{ t("documents.totals.vat") }}</th>
              <th v-if="showCzk" class="text-right">{{ t("documents.totals.baseCzk") }}</th>
              <th v-if="showCzk" class="text-right">{{ t("documents.totals.vatCzk") }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in totals.recap" :key="row.vatRate" data-test="recap-row">
              <td>{{ formatNumber(row.vatRate, locale, 2) }} %</td>
              <td class="text-right tabular-nums">{{ money(row.base) }}</td>
              <td class="text-right tabular-nums">{{ money(row.vat) }}</td>
              <td v-if="showCzk" class="text-right tabular-nums">{{ czk(row.baseCzk) }}</td>
              <td v-if="showCzk" class="text-right tabular-nums">{{ czk(row.vatCzk) }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <dl class="ml-auto grid max-w-sm grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-sm" :class="{ 'opacity-60': pending }">
        <dt>{{ t("documents.totals.base") }}</dt>
        <dd class="text-right tabular-nums">{{ money(totals.base) }}</dd>
        <dt>{{ t("documents.totals.vat") }}</dt>
        <dd class="text-right tabular-nums">{{ money(totals.vat) }}</dd>
        <dt>{{ t("documents.totals.total") }}</dt>
        <dd class="text-right tabular-nums">{{ money(totals.total) }}</dd>
        <template v-if="parseCents(totals.rounding) !== 0n">
          <dt>{{ t("documents.totals.rounding") }}</dt>
          <dd class="text-right tabular-nums">{{ money(totals.rounding) }}</dd>
        </template>
        <dt class="font-semibold" data-test="payable-label">{{ overpaid ? t("documents.totals.overpaid") : t("documents.totals.payable") }}</dt>
        <dd class="text-right text-base font-semibold tabular-nums" :class="{ 'text-purple-700 dark:text-purple-300': overpaid }" data-test="payable">
          {{ money(overpaid ? negate(totals.payable) : totals.payable) }}
        </dd>
        <template v-if="foreign && totals.totalCzk !== null">
          <dt class="text-gray-600 dark:text-gray-400">
            {{ t("documents.totals.totalCzk") }}
            <span v-if="exchangeRate" class="text-xs">({{ t("documents.totals.atRate", { rate: formatNumber(exchangeRate, locale, 4) }) }})</span>
          </dt>
          <dd class="text-right tabular-nums text-gray-600 dark:text-gray-400" data-test="total-czk">{{ czk(totals.totalCzk) }}</dd>
        </template>
      </dl>
    </template>
    <p v-else-if="!pending" class="text-sm text-gray-500">{{ t("documents.totals.none") }}</p>
  </section>
</template>
