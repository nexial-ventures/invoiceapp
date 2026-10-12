<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { FieldErrors } from "@/api/types";
import FormField from "@/components/form/FormField.vue";
import { useErrorText } from "@/composables/useAction";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { reasonKey } from "@/lib/formErrors";
import type { Contact } from "@/features/contacts/types";
import MetadataFields from "@/features/metadata/components/MetadataFields.vue";
import type { Category } from "@/features/settings/types";
import { docSign, isCorrection } from "../docTypes";
import { applyContact, defaultVatRate, toComputeRequest, toInput, validateDocument, type DocumentDraft, type DraftContext } from "../form";
import { formatMoney } from "@/lib/format";
import { enforceVatMode, splitLineErrors } from "../lines";
import { useDocumentStore } from "../store";
import type { Document, RelatedDocument } from "../types";
import { useCompute } from "../useCompute";
import { useIndicativeRate } from "../useIndicativeRate";
import ContactPicker from "./ContactPicker.vue";
import DocumentHeaderFields from "./DocumentHeaderFields.vue";
import LineEditor from "./LineEditor.vue";
import RelatedDocumentPicker from "./RelatedDocumentPicker.vue";
import SimplifiedLimitNote from "./SimplifiedLimitNote.vue";
import TotalsPanel from "./TotalsPanel.vue";

const props = defineProps<{
  initial: DocumentDraft;
  ctx: DraftContext;
  categories: Category[];
  docId?: string;
  /** The linked document of an edited draft (label for the import's picker). */
  parent?: RelatedDocument | null;
}>();
const emit = defineEmits<{ saved: [doc: Document] }>();

const { t, locale } = useI18n();
const errorText = useErrorText();
const store = useDocumentStore();
const draft = ref<DocumentDraft>(props.initial);
const { fieldErrors, error, submitting, submit } = useFormSubmit();
const correction = computed(() => isCorrection(draft.value.docType));
/** A native correction keeps its original's customer, currency and rate. */
const lockedCorrection = computed(() => correction.value && !draft.value.imported);
const indicative = useIndicativeRate(() => ({
  // A native correction keeps its original's rate, and a filled-in manual rate wins: nothing to look up.
  currency: lockedCorrection.value || draft.value.exchangeRate.trim() ? "" : draft.value.currency,
  date: draft.value.taxPointDate || draft.value.issueDate,
}));
const compute = useCompute(() => toComputeRequest(draft.value, indicative.rate.value?.rate ?? null, props.docId));

watch(
  () => draft.value.vatMode,
  (mode) => (draft.value.lines = enforceVatMode(draft.value.lines, mode)),
);

// Server line errors point at indexes; once lines move they would land on the wrong row.
watch(
  () => draft.value.lines.map((l) => l.key).join(),
  () => {
    fieldErrors.value = Object.fromEntries(Object.entries(fieldErrors.value).filter(([k]) => !k.startsWith("lines.")));
  },
);

const vatLocked = computed(() => draft.value.vatMode === "non_payer");
const vatOptions = computed(() =>
  vatLocked.value ? ["0"] : props.ctx.vatRates.filter((r) => r.active).map((r) => r.rate),
);
const newLineVat = computed(() => defaultVatRate(props.ctx.vatRates, draft.value.vatMode));

const saveErrors = computed(() => splitLineErrors(fieldErrors.value));
const computeErrors = computed(() => splitLineErrors(compute.fieldErrors.value));
const headerErrors = computed<FieldErrors>(() => ({ ...computeErrors.value.header, ...saveErrors.value.header }));
const lineErrors = computed(() => {
  const out: Record<number, FieldErrors> = {};
  draft.value.lines.forEach((_, i) => {
    const merged = { ...computeErrors.value.lines[i], ...saveErrors.value.lines[i] };
    if (Object.keys(merged).length) out[i] = merged;
  });
  return out;
});

const bases = computed(() =>
  draft.value.lines.map((line, i) => {
    const res = compute.result.value?.lines[i];
    return res && res.kind === line.kind && "base" in res
      ? formatMoney(res.base, draft.value.currency, locale.value)
      : null;
  }),
);

const computeError = computed(() => {
  if (compute.error.value) return errorText(compute.error.value);
  return Object.keys(compute.fieldErrors.value).length ? t("documents.totals.fixErrors") : null;
});

function onPick(contact: Contact) {
  draft.value = applyContact(draft.value, contact, props.ctx);
}

async function onSubmit() {
  await submit(
    () => validateDocument(draft.value, props.ctx.fieldDefs),
    async () => {
      emit("saved", await store.save(toInput(draft.value, props.ctx.fieldDefs, !props.docId), props.docId));
    },
  );
}
</script>

<template>
  <form class="space-y-6" novalidate @submit.prevent="onSubmit">
    <section v-if="draft.imported" class="card space-y-3" data-test="import-section">
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("documents.import.hint") }}</p>
      <div class="grid gap-4 sm:grid-cols-2">
        <FormField :label="t('documents.import.number')" for="doc-number" :error="headerErrors.number">
          <input id="doc-number" v-model="draft.number" maxlength="40" class="input font-mono" :class="{ 'input-error': headerErrors.number }" autocomplete="off" />
        </FormField>
        <RelatedDocumentPicker
          v-model="draft.relatedDocumentId"
          direction="issued"
          :doc-type="draft.docType"
          :contact-id="draft.contactId"
          :current="parent"
          :exclude-id="docId"
          :error="headerErrors.relatedDocumentId"
        />
      </div>
    </section>

    <section class="card">
      <ContactPicker
        :contact-id="draft.contactId"
        :error="headerErrors.contactId"
        :locked="lockedCorrection"
        :label="draft.docType === 'simplified' ? t('documents.editor.customerOptional') : undefined"
        @pick="onPick"
        @clear="draft.contactId = null"
      />
    </section>

    <section v-if="correction" class="card">
      <FormField :label="t('documents.fields.correctionReason')" for="doc-correctionReason" :error="headerErrors.correctionReason" :hint="t(`documents.editor.correctionHint.${draft.docType}`)">
        <textarea id="doc-correctionReason" v-model="draft.correctionReason" rows="2" maxlength="500" class="input" :class="{ 'input-error': headerErrors.correctionReason }" />
      </FormField>
    </section>

    <section class="card">
      <DocumentHeaderFields
        v-model="draft"
        :errors="headerErrors"
        :bank-accounts="ctx.bankAccounts"
        :company="ctx.company"
        :indicative-rate="indicative.rate.value"
        :indicative-error="indicative.error.value"
        :indicative-loading="indicative.loading.value"
      />
    </section>

    <section class="card space-y-3">
      <h2 class="font-semibold">{{ t("documents.line.title") }}</h2>
      <p v-if="headerErrors.lines" class="text-xs text-red-600 dark:text-red-400" data-test="lines-error">
        {{ headerErrors.lines === "required" ? t("documents.line.atLeastOneItem") : t(reasonKey(headerErrors.lines)) }}
      </p>
      <LineEditor
        v-model="draft.lines"
        :vat-options="vatOptions"
        :default-vat-rate="newLineVat"
        :vat-locked="vatLocked"
        :errors="lineErrors"
        :bases="bases"
        :currency="draft.currency"
        :vat-mode="draft.vatMode"
        :related-document-id="draft.relatedDocumentId"
      />
    </section>

    <TotalsPanel
      :totals="compute.result.value?.totals ?? null"
      :currency="draft.currency"
      :sign="docSign(draft.docType)"
      :exchange-rate="draft.exchangeRate || indicative.rate.value?.rate || null"
      :pending="compute.pending.value"
      :error="computeError"
    />

    <SimplifiedLimitNote :doc-type="draft.docType" :currency="draft.currency" :totals="compute.result.value?.totals" />

    <section class="card grid gap-4 sm:grid-cols-2">
      <FormField :label="t('documents.fields.headerNote')" for="doc-headerNote" :error="headerErrors.headerNote">
        <textarea id="doc-headerNote" v-model="draft.headerNote" rows="3" maxlength="2000" class="input" />
      </FormField>
      <FormField :label="t('documents.fields.footerNote')" for="doc-footerNote" :error="headerErrors.footerNote">
        <textarea id="doc-footerNote" v-model="draft.footerNote" rows="3" maxlength="2000" class="input" />
      </FormField>
    </section>

    <section class="card">
      <MetadataFields v-model="draft.meta" direction="issued" :categories="categories" :field-defs="ctx.fieldDefs" :errors="headerErrors" id-prefix="doc" />
    </section>

    <p v-if="error" role="alert" class="alert-error" data-test="form-error">{{ error }}</p>

    <div class="flex flex-wrap items-center gap-2">
      <button type="submit" class="btn btn-primary" :disabled="submitting" data-test="save-document">
        {{ submitting ? t("common.saving") : t("documents.editor.saveDraft") }}
      </button>
      <slot name="actions" />
    </div>
  </form>
</template>
