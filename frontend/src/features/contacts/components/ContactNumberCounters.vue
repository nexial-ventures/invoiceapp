<script setup lang="ts">
import { onMounted } from "vue";
import { useI18n } from "vue-i18n";
import type { DocType } from "@/api/types";
import { useAction } from "@/composables/useAction";
import CountersEditor from "@/features/settings/components/CountersEditor.vue";
import { useSessionStore } from "@/stores/session";
import { useContactNumberingStore } from "../store";

/** The customer's client sequences per issued series with a client pattern; counters editable by admins. */
const props = defineProps<{ contactId: string; hasCode: boolean }>();

const { t } = useI18n();
const session = useSessionStore();
const store = useContactNumberingStore();
const { error, run } = useAction();
const currentYear = new Date().getFullYear();

const saveFor = (docType: DocType) => (year: number, lastNumber: number) => store.setCounter(docType, year, lastNumber);

onMounted(() => void run(() => store.load(props.contactId)));
</script>

<template>
  <section class="card space-y-4" data-test="contact-counters">
    <h2 class="font-semibold">{{ t("numbering.contact.countersTitle") }}</h2>
    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    <p v-else-if="!store.series.length" class="text-sm text-gray-600 dark:text-gray-400" data-test="no-client-series">
      {{ t("numbering.contact.noClientSeries") }}
    </p>
    <p v-else-if="!hasCode" class="text-sm text-gray-600 dark:text-gray-400" data-test="no-code">{{ t("numbering.contact.noCode") }}</p>

    <div v-for="s in store.series" :key="s.docType" class="space-y-2 border-t border-gray-200 pt-3 dark:border-gray-800" :data-test="`contact-series-${s.docType}`">
      <h3 class="font-medium">{{ t(`settings.numberSeries.docTypes.${s.docType}`) }}</h3>
      <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
        <dt class="text-gray-600 dark:text-gray-400">{{ t("numbering.clientPattern.label") }}</dt>
        <dd class="font-mono">{{ s.clientPattern }}</dd>
        <dt class="text-gray-600 dark:text-gray-400">{{ t("numbering.contact.nextNumber") }}</dt>
        <dd class="font-mono" data-test="contact-next-number">{{ s.nextNumberPreview ?? "—" }}</dd>
      </dl>
      <CountersEditor :counters="s.counters" :current-year="currentYear" :save-counter="saveFor(s.docType)" :readonly="!session.can('settings')" />
    </div>
  </section>
</template>
