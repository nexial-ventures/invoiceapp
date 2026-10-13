<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { ISSUED_SERIES, RECEIVED_SERIES } from "@/api/types";
import { useAction } from "@/composables/useAction";
import NumberSeriesCard from "../components/NumberSeriesCard.vue";
import { useNumberSeriesStore } from "../stores";
import type { NumberSeries } from "../types";

const { t } = useI18n();
const store = useNumberSeriesStore();
const { error, run } = useAction();
const currentYear = new Date().getFullYear();

/** Series of one direction in display order; keys the server does not return are skipped. */
const pick = (keys: readonly string[]) =>
  keys.map((k) => store.series.find((s) => s.docType === k)).filter((s): s is NumberSeries => !!s);
const groups = computed(() => [
  { key: "issued", series: pick(ISSUED_SERIES) },
  { key: "received", series: pick(RECEIVED_SERIES) },
]);

const tokens = [
  { token: "{YYYY}", key: "yyyy" },
  { token: "{YY}", key: "yy" },
  { token: "{NNNN}", key: "n" },
] as const;

onMounted(() => void run(store.load));
</script>

<template>
  <div class="space-y-4">
    <aside class="card text-sm">
      <h2 class="mb-2 font-semibold">{{ t("settings.numberSeries.helpTitle") }}</h2>
      <ul class="space-y-1">
        <li v-for="tok in tokens" :key="tok.key">
          <code class="rounded bg-gray-100 px-1 font-mono dark:bg-gray-800">{{ tok.token }}</code>
          — {{ t(`settings.numberSeries.tokens.${tok.key}`) }}
        </li>
      </ul>
      <p class="mt-2 text-gray-600 dark:text-gray-400">{{ t("settings.numberSeries.helpRules") }}</p>
      <p class="mt-2 text-gray-600 dark:text-gray-400" data-test="client-help">
        <code class="rounded bg-gray-100 px-1 font-mono dark:bg-gray-800">{C}</code>
        — {{ t("numbering.clientPattern.help") }}
      </p>
    </aside>

    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>

    <section v-for="g in groups" v-show="g.series.length" :key="g.key" class="space-y-4" :data-test="`series-group-${g.key}`">
      <h2 class="text-lg font-semibold">{{ t(`settings.numberSeries.groups.${g.key}`) }}</h2>
      <NumberSeriesCard v-for="s in g.series" :key="s.docType" :series="s" :current-year="currentYear" />
    </section>
  </div>
</template>
