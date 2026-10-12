<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { ApiError } from "@/api/client";
import { formatDate, formatNumber } from "@/lib/format";
import type { ExchangeRate } from "../types";

/** State of `useIndicativeRate` as a one-line note; `messageKey` words the found rate (when it gets fixed). */
const props = defineProps<{
  rate: ExchangeRate | null;
  error: unknown;
  loading: boolean;
  currency: string;
  messageKey: string;
}>();

const { t, locale } = useI18n();

/** ČNB not reachable or not listing the currency: a note only, the manual rate or the server covers it. */
const note = computed(() => {
  if (props.loading) return t("documents.editor.indicativeRateLoading");
  const r = props.rate;
  if (r) return t(props.messageKey, { date: formatDate(r.date, locale.value), rate: formatNumber(r.rate, locale.value, 6) });
  if (props.error instanceof ApiError && props.error.status === 404) {
    return t("documents.editor.indicativeRateUnknown", { currency: props.currency });
  }
  return props.error ? t("documents.editor.indicativeRateUnavailable") : null;
});
</script>

<template>
  <p v-if="note" class="text-xs text-gray-600 dark:text-gray-400" data-test="indicative-rate">{{ note }}</p>
</template>
