<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { formatDate } from "@/lib/format";
import type { PreviewEntry } from "../types";

/** Preview cell: the issue, tax point and due dates the entry carries. */
defineProps<{ entry: Pick<PreviewEntry, "issueDate" | "taxPointDate" | "dueDate"> }>();

const { t, locale } = useI18n();
</script>

<template>
  <div v-if="entry.issueDate">{{ t("imports.preview.issueDate", { date: formatDate(entry.issueDate, locale) }) }}</div>
  <div v-if="entry.taxPointDate">{{ t("imports.preview.taxPointDate", { date: formatDate(entry.taxPointDate, locale) }) }}</div>
  <div v-if="entry.dueDate">{{ t("imports.preview.dueDate", { date: formatDate(entry.dueDate, locale) }) }}</div>
</template>
