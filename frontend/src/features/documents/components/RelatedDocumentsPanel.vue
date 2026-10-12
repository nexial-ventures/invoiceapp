<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { signed } from "@/lib/money";
import type { Direction } from "@/api/types";
import { docSign } from "../docTypes";
import { formatMoney } from "@/lib/format";
import { detailLocation } from "../routes";
import type { RelatedDocument } from "../types";
import StatusBadges from "./StatusBadges.vue";

/** `parent`: what this document refers to; `children`: documents referring to it. */
const props = defineProps<{ parent: RelatedDocument | null; children: RelatedDocument[]; /** Links stay within one direction. */ direction?: Direction }>();
const { t, locale } = useI18n();

const rows = computed(() => [
  ...(props.parent ? [{ role: "parent" as const, doc: props.parent }] : []),
  ...props.children.map((doc) => ({ role: "child" as const, doc })),
]);
// Each related document carries its own currency (a received DDPP may differ from its proforma).
const money = (d: RelatedDocument) =>
  formatMoney(signed(d.payable, docSign(d.docType)), d.currency, locale.value);
const typeLabel = (d: RelatedDocument) => t(props.direction === "received" ? `received.docTypes.${d.docType}` : `documents.docTypes.${d.docType}`);
</script>

<template>
  <section v-if="rows.length" class="card space-y-2" data-test="related-documents">
    <h2 class="font-semibold">{{ t("documents.related.title") }}</h2>
    <ul class="divide-y divide-gray-100 dark:divide-gray-800">
      <li v-for="{ role, doc } in rows" :key="doc.id" class="flex flex-wrap items-center gap-x-3 gap-y-1 py-2 text-sm" :data-test="`related-${role}`">
        <span class="text-xs text-gray-500">{{ t(`documents.related.${role}`) }}</span>
        <RouterLink :to="detailLocation(doc.id, direction)" class="font-medium text-blue-600 hover:underline dark:text-blue-400">
          {{ typeLabel(doc) }} <span class="font-mono">{{ doc.number ?? t("documents.draftNumber") }}</span>
        </RouterLink>
        <StatusBadges :status="doc.status" :payment-state="null" :overdue="false" :direction="direction" />
        <span class="ml-auto tabular-nums">{{ money(doc) }}</span>
      </li>
    </ul>
  </section>
</template>
