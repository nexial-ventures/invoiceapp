<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { formatMoney } from "@/lib/format";
import { isNegative } from "@/lib/money";
import type { Overview } from "../types";

defineProps<{ overview: Overview }>();
const { t, locale } = useI18n();
const money = (v: string) => formatMoney(v, "CZK", locale.value);
</script>

<template>
  <div class="grid gap-4 sm:grid-cols-3">
    <div v-for="side in ['income', 'expense'] as const" :key="side" class="card space-y-2" :data-test="`tile-${side}`">
      <p class="text-sm text-gray-600 dark:text-gray-400">
        {{ t(`overview.tiles.${side}`) }}
        <span class="text-xs">({{ t(overview.vatPayer ? "overview.tiles.basisPayer" : "overview.tiles.basisTotal") }})</span>
      </p>
      <p class="text-2xl font-semibold tabular-nums" data-test="tile-amount">{{ money(overview[side].amount) }}</p>
      <dl class="grid grid-cols-[auto_1fr] gap-x-3 text-xs text-gray-600 dark:text-gray-400">
        <template v-if="overview.vatPayer">
          <dt>{{ t("overview.tiles.base") }}</dt>
          <dd class="text-right tabular-nums">{{ money(overview[side].base) }}</dd>
          <dt>{{ t("overview.tiles.vat") }}</dt>
          <dd class="text-right tabular-nums">{{ money(overview[side].vat) }}</dd>
        </template>
        <dt>{{ t("overview.tiles.total") }}</dt>
        <dd class="text-right tabular-nums">{{ money(overview[side].total) }}</dd>
        <dt>{{ t("overview.tiles.count") }}</dt>
        <dd class="text-right tabular-nums">{{ overview[side].count }}</dd>
      </dl>
    </div>
    <div class="card space-y-2" data-test="tile-difference">
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("overview.tiles.difference") }}</p>
      <p class="text-2xl font-semibold tabular-nums" :class="{ 'text-red-600 dark:text-red-400': isNegative(overview.difference) }" data-test="tile-amount">
        {{ money(overview.difference) }}
      </p>
      <p class="text-xs text-gray-600 dark:text-gray-400">{{ t("overview.tiles.differenceHint") }}</p>
    </div>
  </div>
</template>
