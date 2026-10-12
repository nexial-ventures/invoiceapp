<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { useAction } from "@/composables/useAction";
import { todayIso } from "@/features/documents/form";
import { detailLocation, listDocTypeOf, listLocation } from "@/features/documents/routes";
import { useDocumentStore } from "@/features/documents/store";
import type { Document, RelatedDocument } from "@/features/documents/types";
import { useMetadataSettings } from "@/features/metadata/useMetadataSettings";
import { useVatRatesStore } from "@/features/settings/stores";
import ReceivedEditorForm from "../components/ReceivedEditorForm.vue";
import { newReceivedDraft, toReceivedDraft, type ReceivedContext, type ReceivedDraft } from "../form";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const vatRates = useVatRatesStore();
const metadata = useMetadataSettings(() => "received");
const store = useDocumentStore();
const { error, run } = useAction();

const id = computed(() => (typeof route.params.id === "string" ? route.params.id : undefined));
/** `/received/new?docType=proforma`; anything unknown starts an invoice. */
const newDocType = computed(() => listDocTypeOf(route.query.docType));
const initial = ref<ReceivedDraft | null>(null);
const ctx = ref<ReceivedContext | null>(null);
const parent = ref<RelatedDocument | null>(null);

watch(
  () => [id.value, newDocType.value] as const,
  ([docId]) =>
    void run(async () => {
      initial.value = null;
      // The instance is reused between /new and /:id/edit: nothing of a previously loaded document may leak.
      parent.value = null;
      await Promise.all([vatRates.ensure(), metadata.load()]);
      ctx.value = { vatRates: vatRates.items, fieldDefs: metadata.fieldDefs.value, today: todayIso() };
      if (!docId) {
        initial.value = newReceivedDraft(ctx.value, newDocType.value);
        return;
      }
      await store.load(docId);
      if (!store.doc) return;
      if (store.doc.direction !== "received") {
        await router.replace(detailLocation(docId));
        return;
      }
      parent.value = store.doc.parent;
      initial.value = toReceivedDraft(store.doc, ctx.value.fieldDefs);
    }),
  { immediate: true },
);

async function onSaved(doc: Document) {
  await router.push(detailLocation(doc.id, "received"));
}

const docType = computed(() => initial.value?.docType ?? newDocType.value);
const back = computed(() => (id.value ? detailLocation(id.value, "received") : listLocation(docType.value, "received")));
</script>

<template>
  <section class="space-y-4">
    <RouterLink :to="back" class="text-sm text-blue-600 hover:underline dark:text-blue-400">
      ← {{ id ? t("documents.detail.backToDetail") : t("documents.detail.backToList") }}
    </RouterLink>
    <h1 class="text-2xl font-semibold">{{ id ? t(`received.editTitle.${docType}`) : t(`received.newTitle.${docType}`) }}</h1>

    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    <p v-else-if="!initial" class="text-sm text-gray-500">{{ t("common.loading") }}</p>

    <ReceivedEditorForm
      v-if="initial && ctx"
      :key="id ?? `new-${newDocType}`"
      :initial="initial"
      :ctx="ctx"
      :categories="metadata.categories.value"
      :doc-id="id"
      :parent="parent"
      @saved="onSaved"
    >
      <template #actions>
        <RouterLink :to="back" class="btn">{{ t("common.cancel") }}</RouterLink>
      </template>
    </ReceivedEditorForm>
  </section>
</template>
