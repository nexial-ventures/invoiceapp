<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { formatMoney } from "@/lib/format";
import { parseCents } from "@/lib/money";
import type { OverviewCategory } from "../types";

const props = defineProps<{ title: string; items: OverviewCategory[]; tone: "income" | "expense" }>();
const { t, locale } = useI18n();

/** Share of the largest positive amount, for the bar width (display only). */
function share(amount: string): number {
  const max = props.items.reduce((m, c) => Math.max(m, Number(parseCents(c.amount) ?? 0n)), 0);
  const v = Number(parseCents(amount) ?? 0n);
  return max > 0 && v > 0 ? Math.round((v / max) * 100) : 0;
}
</script>

<template>
  <div class="card space-y-2" :data-test="`categories-${tone}`">
    <h3 class="font-semibold">{{ title }}</h3>
    <p v-if="items.length === 0" class="text-sm text-gray-500">{{ t("overview.categories.none") }}</p>
    <ul v-else class="space-y-2 text-sm">
      <li v-for="c in items" :key="c.id ?? 'none'" data-test="category">
        <div class="flex justify-between gap-3">
          <span class="truncate" :class="{ 'text-gray-500 italic': !c.id }">{{ c.name ?? t("overview.categories.uncategorized") }}</span>
          <span class="shrink-0 tabular-nums">{{ formatMoney(c.amount, "CZK", locale) }}</span>
        </div>
        <div class="mt-1 h-1.5 rounded-full bg-gray-100 dark:bg-gray-800">
          <div
            class="h-1.5 rounded-full"
            :class="tone === 'income' ? 'bg-blue-600 dark:bg-blue-500' : 'bg-orange-600'"
            :style="{ width: `${share(c.amount)}%` }"
          />
        </div>
      </li>
    </ul>
  </div>
</template>
