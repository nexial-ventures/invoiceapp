<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { Document } from "../types";

/**
 * When, by whom and why a document was last unlocked: a banner (with the kept number) on the draft,
 * a small note once it is issued again.
 */
const props = defineProps<{ doc: Pick<Document, "status" | "number" | "imported" | "unlockedAt" | "unlockedBy" | "unlockReason"> }>();

const { t, locale } = useI18n();

const draft = computed(() => props.doc.status === "draft");
const when = computed(() =>
  props.doc.unlockedAt ? new Intl.DateTimeFormat(locale.value, { dateStyle: "medium", timeStyle: "short" }).format(new Date(props.doc.unlockedAt)) : "",
);
const who = computed(() => {
  const by = props.doc.unlockedBy;
  return by ? by.displayName || by.email : t("unlock.unknownUser");
});
const text = computed(() => t("unlock.banner", { when: when.value, who: who.value, reason: props.doc.unlockReason ?? "" }));
</script>

<template>
  <template v-if="doc.unlockedAt">
    <div v-if="draft" role="status" class="rounded-md bg-blue-50 px-3 py-2 text-sm text-blue-800 dark:bg-blue-950/50 dark:text-blue-300" data-test="unlocked-banner">
      <p class="font-medium">{{ t("unlock.bannerTitle") }}</p>
      <p>{{ text }}</p>
      <p v-if="doc.number && !doc.imported" data-test="kept-number">{{ t("unlock.keptNumber", { number: doc.number }) }}</p>
    </div>
    <p v-else class="text-sm text-gray-600 dark:text-gray-400" data-test="unlocked-note">{{ text }}</p>
  </template>
</template>
