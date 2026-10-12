<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { useAction } from "@/composables/useAction";
import { useMetadataSettings } from "@/features/metadata/useMetadataSettings";
import { useBankAccountsStore, useCompanyStore, useVatRatesStore } from "@/features/settings/stores";
import InvoiceEditorForm from "../components/InvoiceEditorForm.vue";
import { isNativeNewType } from "../docTypes";
import { newDocumentDraft, toDraft, todayIso, type DocumentDraft, type DraftContext } from "../form";
import { detailLocation, editLocation, listDocTypeOf, listLocation } from "../routes";
import { useDocumentStore } from "../store";
import type { Document, EditableDocType, RelatedDocument } from "../types";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const company = useCompanyStore();
const vatRates = useVatRatesStore();
const bankAccounts = useBankAccountsStore();
const metadata = useMetadataSettings(() => "issued");
const store = useDocumentStore();
const { error, run } = useAction();

const id = computed(() => (typeof route.params.id === "string" ? route.params.id : undefined));
/** `&imported=1`: enter an existing document (any type, own number, no rendered PDF). */
const newImported = computed(() => route.query.imported === "1");
/** `/invoices/new?docType=proforma|simplified` starts that type; natively everything else an invoice. */
const newDocType = computed<EditableDocType>(() => {
  const type = listDocTypeOf(route.query.docType);
  return newImported.value || isNativeNewType(type) ? type : "invoice";
});
const initial = ref<DocumentDraft | null>(null);
const ctx = ref<DraftContext | null>(null);
const parent = ref<RelatedDocument | null>(null);

watch(
  () => [id.value, newDocType.value, newImported.value] as const,
  ([docId]) =>
    void run(async () => {
      initial.value = null;
      // The instance is reused between /new and /:id/edit: nothing of a previously loaded document may leak.
      parent.value = null;
      await Promise.all([
        company.ensure(),
        vatRates.ensure(),
        bankAccounts.ensure(),
        metadata.load(),
      ]);
      if (!company.company) return;
      ctx.value = {
        company: company.company,
        vatRates: vatRates.items,
        bankAccounts: bankAccounts.items,
        fieldDefs: metadata.fieldDefs.value,
        today: todayIso(),
      };
      if (!docId) {
        initial.value = newDocumentDraft(ctx.value, newDocType.value, newImported.value);
        return;
      }
      await store.load(docId);
      if (store.doc?.direction === "received") {
        await router.replace(editLocation(docId, "received"));
        return;
      }
      if (store.doc && store.doc.status !== "draft") {
        // Issued documents are locked; the detail view has the actions still allowed.
        await router.replace(detailLocation(docId));
        return;
      }
      if (store.doc) {
        parent.value = store.doc.parent;
        initial.value = toDraft(store.doc, ctx.value.fieldDefs);
      }
    }),
  { immediate: true },
);

async function onSaved(doc: Document) {
  await router.push(detailLocation(doc.id));
}

const docType = computed<EditableDocType>(() => initial.value?.docType ?? newDocType.value);
const imported = computed(() => initial.value?.imported ?? newImported.value);
const title = computed(() => {
  if (imported.value) return t(`documents.import.title.${docType.value}`);
  return id.value ? t(`documents.editor.editTitle.${docType.value}`) : t(`documents.editor.newTitle.${docType.value}`);
});
const back = computed(() => (id.value ? detailLocation(id.value) : listLocation(docType.value)));
</script>

<template>
  <section class="space-y-4">
    <RouterLink :to="back" class="text-sm text-blue-600 hover:underline dark:text-blue-400">
      ← {{ id ? t("documents.detail.backToDetail") : t("documents.detail.backToList") }}
    </RouterLink>
    <h1 class="text-2xl font-semibold">{{ title }}</h1>

    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    <p v-else-if="!initial" class="text-sm text-gray-500">{{ t("common.loading") }}</p>

    <InvoiceEditorForm
      v-if="initial && ctx"
      :key="id ?? `new-${newDocType}-${newImported}`"
      :initial="initial"
      :ctx="ctx"
      :categories="metadata.categories.value"
      :doc-id="id"
      :parent="parent"
      @saved="onSaved"
    >
      <template #actions>
        <RouterLink :to="back" class="btn">
          {{ t("common.cancel") }}
        </RouterLink>
      </template>
    </InvoiceEditorForm>
  </section>
</template>
