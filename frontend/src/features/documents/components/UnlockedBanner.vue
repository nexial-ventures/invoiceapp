<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { Document } from "../types";

/** On a draft that was issued before: when, by whom and why it was unlocked, and the number it keeps. */
const props = defineProps<{ doc: Pick<Document, "status" | "number" | "imported" | "unlockedAt" | "unlockedBy" | "unlockReason"> }>();

const { t, locale } = useI18n();

const show = computed(() => props.doc.status === "draft" && !!props.doc.unlockedAt);
const when = computed(() =>
  props.doc.unlockedAt ? new Intl.DateTimeFormat(locale.value, { dateStyle: "medium", timeStyle: "short" }).format(new Date(props.doc.unlockedAt)) : "",
);
const who = computed(() => {
  const by = props.doc.unlockedBy;
  return by ? by.displayName || by.email : t("unlock.unknownUser");
});
</script>

<template>
  <div v-if="show" role="status" class="rounded-md bg-blue-50 px-3 py-2 text-sm text-blue-800 dark:bg-blue-950/50 dark:text-blue-300" data-test="unlocked-banner">
    <p class="font-medium">{{ t("unlock.bannerTitle") }}</p>
    <p>{{ t("unlock.banner", { when, who, reason: doc.unlockReason ?? "" }) }}</p>
    <p v-if="doc.number && !doc.imported" data-test="kept-number">{{ t("unlock.keptNumber", { number: doc.number }) }}</p>
  </div>
</template>
