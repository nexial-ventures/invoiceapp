<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { useAction } from "@/composables/useAction";
import { overviewApi } from "../api";
import BarChart from "../components/BarChart.vue";
import CategoryBreakdown from "../components/CategoryBreakdown.vue";
import OpenDocuments from "../components/OpenDocuments.vue";
import OverviewTiles from "../components/OverviewTiles.vue";
import PeriodSelector from "../components/PeriodSelector.vue";
import VatTable from "../components/VatTable.vue";
import { fromQuery, toQuery, type Period } from "../period";
import type { Overview } from "../types";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { error, run } = useAction();

/** The period lives in the URL (shareable, survives reloads); no query → the current month. */
const period = computed(() => fromQuery(route.query));
const overview = ref<Overview | null>(null);
const loading = ref(false);
let latest = 0;

async function load(from: string, to: string) {
  const mine = ++latest;
  loading.value = true;
  await run(async () => {
    const o = await overviewApi.get(from, to);
    // A slower answer for a period the user already left is dropped.
    if (mine === latest) overview.value = o;
  });
  if (mine === latest) loading.value = false;
}

// Each source compared on its own: switching to "custom" with the same dates does not refetch.
watch([() => period.value.from, () => period.value.to], ([from, to]) => void load(from, to), { immediate: true });

const onPeriod = (p: Period) => void router.replace({ query: toQuery(p) });
</script>

<template>
  <section class="space-y-6">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <h1 class="text-2xl font-semibold">{{ t("overview.title") }}</h1>
      <PeriodSelector :model-value="period" @update:model-value="onPeriod" />
    </div>
    <p class="text-xs text-gray-500">{{ t("overview.basis") }}</p>

    <p v-if="error" role="alert" class="alert-error" data-test="overview-error">{{ error }}</p>

    <div v-if="overview" class="space-y-6" :class="{ 'opacity-60': loading }" :aria-busy="loading" data-test="overview">
      <OverviewTiles :overview="overview" />

      <div class="card">
        <h2 class="mb-3 font-semibold">{{ t("overview.chart.title") }}</h2>
        <BarChart :months="overview.months" />
      </div>

      <div v-if="overview.vatPayer" class="space-y-2" data-test="vat-block">
        <h2 class="font-semibold">{{ t("overview.vat.title") }}</h2>
        <VatTable :vat="overview.vat" />
      </div>

      <div class="grid gap-4 md:grid-cols-2">
        <OpenDocuments :open="overview.receivables" direction="issued" :today="overview.today" />
        <OpenDocuments :open="overview.payables" direction="received" :today="overview.today" />
      </div>

      <div class="grid gap-4 md:grid-cols-2">
        <CategoryBreakdown :title="t('overview.categories.income')" :items="overview.categories.income" tone="income" />
        <CategoryBreakdown :title="t('overview.categories.expense')" :items="overview.categories.expense" tone="expense" />
      </div>
    </div>
    <p v-else-if="loading" class="text-gray-500">{{ t("overview.loading") }}</p>
  </section>
</template>
