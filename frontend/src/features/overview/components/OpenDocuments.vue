<script setup lang="ts">
import { useI18n } from "vue-i18n";
import type { Direction } from "@/api/types";
import { formatDate, formatMoney } from "@/lib/format";
import type { OverviewOpen } from "../types";

defineProps<{ open: OverviewOpen; direction: Direction; today: string }>();
const { t, locale } = useI18n();
const money = (v: string) => formatMoney(v, "CZK", locale.value);
const detail = (direction: Direction, id: string) => ({ name: direction === "issued" ? "invoice-detail" : "received-detail", params: { id } });
</script>

<template>
  <div class="card space-y-3" :data-test="`open-${direction}`">
    <div class="flex items-baseline justify-between gap-2">
      <h3 class="font-semibold">{{ t(direction === "issued" ? "overview.open.receivables" : "overview.open.payables") }}</h3>
      <span class="text-xs text-gray-500">{{ t("overview.open.asOf", { date: formatDate(today, locale) }) }}</span>
    </div>
    <dl class="grid grid-cols-[1fr_auto_auto] gap-x-4 gap-y-1 text-sm">
      <dt>{{ t("overview.open.unpaid") }}</dt>
      <dd class="text-right text-gray-500 tabular-nums">{{ open.count }}×</dd>
      <dd class="text-right font-medium tabular-nums" data-test="open-amount">{{ money(open.amount) }}</dd>
      <dt>{{ t("overview.open.overdue") }}</dt>
      <dd class="text-right text-gray-500 tabular-nums">{{ open.overdueCount }}×</dd>
      <dd class="text-right font-medium tabular-nums" :class="{ 'text-red-600 dark:text-red-400': open.overdueCount > 0 }" data-test="open-overdue">
        {{ money(open.overdueAmount) }}
      </dd>
      <dt class="text-gray-600 dark:text-gray-400">{{ t("overview.open.advances") }}</dt>
      <dd class="text-right text-gray-500 tabular-nums">{{ open.advances.count }}×</dd>
      <dd class="text-right tabular-nums text-gray-600 dark:text-gray-400" data-test="open-advances">{{ money(open.advances.amount) }}</dd>
    </dl>
    <div v-if="open.topOverdue.length > 0">
      <p class="mb-1 text-xs font-medium text-gray-600 uppercase dark:text-gray-400">{{ t("overview.open.topOverdue") }}</p>
      <ul class="divide-y divide-gray-100 text-sm dark:divide-gray-800">
        <li v-for="d in open.topOverdue" :key="d.id" class="flex items-baseline justify-between gap-3 py-1" data-test="open-top">
          <span class="min-w-0 truncate">
            <RouterLink :to="detail(direction, d.id)" class="font-mono hover:underline">{{ d.number ?? "—" }}</RouterLink>
            <span class="text-gray-600 dark:text-gray-400"> · {{ d.counterparty ?? "—" }}</span>
          </span>
          <span class="shrink-0 text-right tabular-nums">
            <span class="mr-2 text-xs text-red-600 dark:text-red-400">{{ formatDate(d.dueDate, locale) }}</span>{{ money(d.remaining) }}
          </span>
        </li>
      </ul>
    </div>
  </div>
</template>
