<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import UsageBar from "@/components/UsageBar.vue";
import { formatBytes } from "@/lib/bytes";
import type { DesignQuota } from "../../types";

const props = defineProps<{ quota: DesignQuota }>();
const { t, locale } = useI18n();

const label = computed(() => t("design.quota", { used: formatBytes(props.quota.used, locale.value), limit: formatBytes(props.quota.limit, locale.value) }));
</script>

<template>
  <UsageBar :used="quota.used" :limit="quota.limit" :label="label" :hint="t('design.quotaHint')" test-id="design-quota" />
</template>
