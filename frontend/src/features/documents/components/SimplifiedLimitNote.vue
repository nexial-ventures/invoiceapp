<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { SIMPLIFIED_LIMIT_CZK, simplifiedOverLimit } from "../docTypes";
import { formatMoney } from "@/lib/format";
import type { Totals } from "../types";

/** Warning only: the server never refuses a simplified document above the limit. */
const props = defineProps<{ docType: string; currency: string; totals: Totals | null | undefined }>();
const { t, locale } = useI18n();
const show = computed(() => simplifiedOverLimit(props.docType, props.currency, props.totals));
</script>

<template>
  <p v-if="show" role="status" class="rounded-md bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-950/50 dark:text-amber-300" data-test="simplified-limit">
    {{ t("documents.editor.simplifiedLimit", { limit: formatMoney(String(SIMPLIFIED_LIMIT_CZK), "CZK", locale) }) }}
  </p>
</template>
