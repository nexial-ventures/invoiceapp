<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { formatMoney, formatMonth } from "@/lib/format";
import { HEIGHT, layout, WIDTH } from "../chart";
import type { OverviewMonth } from "../types";

const props = defineProps<{ months: OverviewMonth[] }>();
const { t, locale } = useI18n();

const chart = computed(() => layout(props.months));
const trailing = computed(() => props.months.some((m) => !m.inPeriod));
const money = (v: string) => formatMoney(v, "CZK", locale.value);
</script>

<template>
  <div class="space-y-2">
    <div class="flex flex-wrap items-center gap-4 text-sm text-gray-600 dark:text-gray-400" data-test="chart-legend">
      <span class="flex items-center gap-1.5"><span class="inline-block size-3 rounded-sm bg-blue-600 dark:bg-blue-500" />{{ t("overview.chart.income") }}</span>
      <span class="flex items-center gap-1.5"><span class="inline-block size-3 rounded-sm bg-orange-600" />{{ t("overview.chart.expense") }}</span>
      <span v-if="trailing" class="text-xs" data-test="chart-trailing">{{ t("overview.chart.trailing") }}</span>
    </div>
    <svg :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" class="h-auto w-full" role="img" :aria-label="t('overview.chart.aria')" data-test="chart">
      <line :x1="0" :x2="WIDTH" :y1="chart.zero" :y2="chart.zero" class="stroke-gray-300 dark:stroke-gray-700" stroke-width="1" />
      <g
        v-for="c in chart.columns"
        :key="c.month"
        :opacity="c.inPeriod ? 1 : 0.35"
        data-test="chart-month"
        :data-in-period="c.inPeriod"
      >
        <title>{{ formatMonth(c.month, locale) }} — {{ t("overview.chart.income") }}: {{ money(c.income) }}, {{ t("overview.chart.expense") }}: {{ money(c.expense) }}</title>
        <!-- Transparent hit target bigger than the bars, so the tooltip shows anywhere in the month's slot. -->
        <rect :x="c.center - WIDTH / chart.columns.length / 2" y="0" :width="WIDTH / chart.columns.length" :height="HEIGHT" fill="transparent" />
        <path v-if="c.incomePath" :d="c.incomePath" class="fill-blue-600 dark:fill-blue-500" />
        <path v-if="c.expensePath" :d="c.expensePath" class="fill-orange-600" />
        <text
          :x="c.center"
          :y="HEIGHT - 6"
          text-anchor="middle"
          font-size="11"
          class="fill-gray-500 dark:fill-gray-400"
          :font-weight="c.inPeriod ? 600 : 400"
        >
          {{ formatMonth(c.month, locale, "short") }}
        </text>
      </g>
    </svg>
    <table class="sr-only">
      <caption>{{ t("overview.chart.aria") }}</caption>
      <thead>
        <tr>
          <th scope="col">{{ t("overview.chart.month") }}</th>
          <th scope="col">{{ t("overview.chart.income") }}</th>
          <th scope="col">{{ t("overview.chart.expense") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="m in months" :key="m.month">
          <th scope="row">{{ formatMonth(m.month, locale) }}</th>
          <td>{{ money(m.income) }}</td>
          <td>{{ money(m.expense) }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
