<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useErrorText } from "@/composables/useAction";
import { usePdf } from "@/composables/usePdf";
import { formatBytes } from "@/lib/bytes";
import { fieldErrorsOf, isApiError } from "@/lib/formErrors";
import { documentsApi } from "../api";
import { formatDate } from "../format";
import { checkOriginalFile } from "../original";
import { useDocumentStore } from "../store";
import type { Document } from "../types";
import { useSessionStore } from "@/stores/session";

/** The uploaded original PDF of a received or imported document: upload / replace / delete, open / download. */
const props = defineProps<{ doc: Document }>();

const { t, locale } = useI18n();
const session = useSessionStore();
const store = useDocumentStore();
const errorText = useErrorText();
const pdf = usePdf();
const input = ref<HTMLInputElement | null>(null);
const busy = ref(false);
const error = ref<string | null>(null);

const original = computed(() => props.doc.original);
const fallbackName = computed(() => `${props.doc.number ?? props.doc.id.slice(0, 8)}-original.pdf`);

async function act(action: () => Promise<void>) {
  busy.value = true;
  error.value = null;
  try {
    await action();
  } catch (err) {
    if (fieldErrorsOf(err)?.file) error.value = t("original.notPdf");
    else if (isApiError(err, 422, "quota_exceeded")) error.value = t("original.quotaExceeded", { detail: err.detail ?? "" });
    else error.value = errorText(err);
  } finally {
    busy.value = false;
  }
}

function onFile(event: Event) {
  const el = event.target as HTMLInputElement;
  const file = el.files?.[0];
  el.value = "";
  if (!file) return;
  const problem = checkOriginalFile(file);
  if (problem) {
    error.value = t(`original.${problem}`);
    return;
  }
  void act(() => store.uploadOriginal(file));
}

function remove() {
  if (!window.confirm(t("original.confirmDelete"))) return;
  void act(store.removeOriginal);
}

const openPdf = () => void pdf.open(() => documentsApi.pdf(props.doc.id));
const downloadPdf = () => void pdf.download(() => documentsApi.pdf(props.doc.id, true), fallbackName.value);
</script>

<template>
  <section class="card space-y-3" data-test="original-pdf">
    <div class="flex flex-wrap items-baseline justify-between gap-2">
      <h2 class="font-semibold">{{ t("original.title") }}</h2>
      <span v-if="original" class="text-sm text-gray-600 dark:text-gray-400" data-test="original-info">
        {{ t("original.uploadedAt", { date: formatDate(original.uploadedAt, locale), size: formatBytes(original.size, locale) }) }}
      </span>
      <span v-else class="text-sm text-gray-600 dark:text-gray-400" data-test="original-missing">{{ t("errors.pdfMissing") }}</span>
    </div>
    <input ref="input" type="file" accept="application/pdf,.pdf" class="hidden" data-test="original-input" @change="onFile" />
    <div class="flex flex-wrap items-center gap-2">
      <template v-if="original">
        <button type="button" class="btn" :disabled="pdf.busy.value" data-test="original-open" @click="openPdf">{{ t("pdf.open") }}</button>
        <button type="button" class="btn" :disabled="pdf.busy.value" data-test="original-download" @click="downloadPdf">{{ t("pdf.download") }}</button>
      </template>
      <template v-if="session.can('write')">
        <button type="button" class="btn" :class="{ 'btn-primary': !original }" :disabled="busy" data-test="original-upload" @click="input?.click()">
          {{ original ? t("original.replace") : t("original.upload") }}
        </button>
        <button v-if="original" type="button" class="btn btn-danger" :disabled="busy" data-test="original-delete" @click="remove">{{ t("common.delete") }}</button>
      </template>
      <span v-if="busy" class="text-sm text-gray-500">{{ t("original.uploading") }}</span>
    </div>
    <p class="text-xs text-gray-500 dark:text-gray-400">{{ t("original.hint") }}</p>
    <p v-if="error" role="alert" class="alert-error" data-test="original-error">{{ error }}</p>
    <p v-if="pdf.error.value" role="alert" class="alert-error" data-test="pdf-error">{{ pdf.error.value }}</p>
  </section>
</template>
