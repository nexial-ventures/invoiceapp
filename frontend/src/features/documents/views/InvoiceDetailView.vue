<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { useAction } from "@/composables/useAction";
import { contactsApi } from "@/features/contacts/api";
import { useCompanyStore } from "@/features/settings/stores";
import DocumentActions from "../components/DocumentActions.vue";
import DocumentInfoCard from "../components/DocumentInfoCard.vue";
import DocumentLinesTable from "../components/DocumentLinesTable.vue";
import DocumentPdf from "../components/DocumentPdf.vue";
import OriginalPdfPanel from "../components/OriginalPdfPanel.vue";
import EmailHistory from "@/features/email/components/EmailHistory.vue";
import SendEmailButton from "@/features/email/components/SendEmailButton.vue";
import IsdocDownloadButton from "@/features/isdoc/components/IsdocDownloadButton.vue";
import MetadataCard from "@/features/metadata/components/MetadataCard.vue";
import PartiesCard from "../components/PartiesCard.vue";
import PaymentsPanel from "../components/PaymentsPanel.vue";
import RelatedDocumentsPanel from "../components/RelatedDocumentsPanel.vue";
import SimplifiedLimitNote from "../components/SimplifiedLimitNote.vue";
import StatusBadges from "../components/StatusBadges.vue";
import TotalsPanel from "../components/TotalsPanel.vue";
import UnlockedBanner from "../components/UnlockedBanner.vue";
import UnlockPanel from "../components/UnlockPanel.vue";
import { detailLocation, listLocation } from "../routes";
import { useDocumentStore } from "../store";
import type { PartySnapshot } from "../types";
import { useSessionStore } from "@/stores/session";

const { t } = useI18n();
const session = useSessionStore();
const route = useRoute();
const router = useRouter();
const store = useDocumentStore();
const company = useCompanyStore();
const { error, run } = useAction();

const id = computed(() => String(route.params.id));
/** Drafts have no snapshots yet; show the live parties instead. */
const liveSupplier = ref<PartySnapshot | null>(null);
const liveCustomer = ref<PartySnapshot | null>(null);

async function loadLiveParties(contactId: string | null) {
  await company.ensure();
  liveSupplier.value = company.company;
  liveCustomer.value = contactId ? { ...(await contactsApi.get(contactId)), registration: null, vatPayer: null, web: null } : null;
}

watch(
  id,
  (docId) =>
    void run(async () => {
      await store.load(docId);
      // A received document opened under /invoices/… belongs to the received views.
      if (store.doc?.direction === "received") {
        await router.replace(detailLocation(docId, "received"));
        return;
      }
      if (store.doc?.status === "draft") await loadLiveParties(store.doc.contactId);
    }),
  { immediate: true },
);

const doc = computed(() => (store.doc?.id === id.value && store.doc.direction === "issued" ? store.doc : null));
const isDraft = computed(() => doc.value?.status === "draft");
// A native DDPP belongs to its proforma payment: never unlocked.
const canUnlock = computed(
  () => doc.value?.status === "issued" && session.can("unlockDocument") && (doc.value.docType !== "advance_tax_doc" || doc.value.imported),
);
// A DDPP has no payments of its own.
const hasPayments = computed(() => !isDraft.value && doc.value?.docType !== "advance_tax_doc");
</script>

<template>
  <section class="space-y-4">
    <RouterLink :to="listLocation(doc?.docType ?? 'invoice')" class="text-sm text-blue-600 hover:underline dark:text-blue-400">
      ← {{ t("documents.detail.backToList") }}
    </RouterLink>

    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    <p v-if="!doc && !error" class="text-sm text-gray-500">{{ t("common.loading") }}</p>

    <template v-if="doc">
      <div class="flex flex-wrap items-center gap-3">
        <h1 class="text-2xl font-semibold">
          {{ t(`documents.docTypes.${doc.docType}`) }}
          <span class="font-mono">{{ doc.number ?? t("documents.draftNumber") }}</span>
        </h1>
        <StatusBadges :status="doc.status" :payment-state="doc.paymentState" :overdue="doc.overdue" :sent-at="doc.sentAt" :sign="doc.sign" :doc-type="doc.docType" :imported="doc.imported" />
        <span v-if="doc.settled" class="badge !bg-green-100 !text-green-800 dark:!bg-green-900/50 dark:!text-green-300" data-test="settled-badge">
          {{ t("documents.detail.settled") }}
        </span>
      </div>
      <p v-if="doc.docType === 'advance_tax_doc' && !doc.imported" class="text-sm text-gray-600 dark:text-gray-400" data-test="ddpp-note">{{ t("documents.detail.ddppNote") }}</p>

      <SimplifiedLimitNote :doc-type="doc.docType" :currency="doc.currency" :totals="doc.totals" />

      <UnlockedBanner :doc="doc" />
      <DocumentActions v-if="session.can('write')" :doc="doc" />
      <UnlockPanel v-if="canUnlock" :key="doc.id" :doc="doc" />
      <div v-if="!isDraft" class="flex flex-wrap items-center gap-2">
        <SendEmailButton v-if="session.can('write')" :document-id="doc.id" />
        <IsdocDownloadButton v-if="doc.number" :id="doc.id" :number="doc.number" />
      </div>
      <!-- An imported document is never rendered with our design: its PDF is the uploaded original. -->
      <OriginalPdfPanel v-if="doc.imported" :doc="doc" />
      <DocumentPdf v-else :doc="doc" />

      <PartiesCard :supplier="isDraft ? liveSupplier : doc.supplier" :customer="isDraft ? liveCustomer : doc.customer" />
      <RelatedDocumentsPanel :parent="doc.parent" :children="doc.relatedDocuments" />
      <DocumentInfoCard :doc="doc" />
      <DocumentLinesTable :lines="doc.lines" :currency="doc.currency" />
      <TotalsPanel :totals="doc.totals" :currency="doc.currency" :exchange-rate="doc.exchangeRate" :sign="doc.sign" />
      <PaymentsPanel v-if="hasPayments" :doc="doc" />
      <EmailHistory v-if="!isDraft" :document-id="doc.id" />
      <MetadataCard :doc="doc" />
    </template>
  </section>
</template>
