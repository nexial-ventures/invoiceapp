<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import ErrorDetail from "@/components/ErrorDetail.vue";
import { usePdf } from "@/composables/usePdf";
import { documentsApi } from "../api";
import { formatDate } from "@/lib/format";
import { useDocumentStore } from "../store";
import type { Document } from "../types";

const props = defineProps<{ doc: Document }>();
const { t, locale } = useI18n();
const store = useDocumentStore();
const { busy, error, detail, open, download } = usePdf();

const isDraft = computed(() => props.doc.status === "draft");
// Mirrors the server's naming; only used when Content-Disposition carries no filename.
const fallbackName = computed(() => `${props.doc.number ?? `draft-${props.doc.id.slice(0, 8)}`}.pdf`);

/**
 * An issued document without an archive (e.g. a DDPP whose post-payment render failed) is
 * archived by its first download; reload so the archive time shows. A failed reload only
 * leaves the label stale, so it is not reported over the successful PDF.
 * `action` runs before the first await so `openPdf` keeps the click's user activation.
 */
async function withReload(action: (id: string) => Promise<boolean>) {
  const { id } = props.doc;
  const lazy = !isDraft.value && !props.doc.pdf;
  if ((await action(id)) && lazy && store.doc?.id === id) await store.load(id).catch(() => {});
}

const openPdf = () => void withReload((id) => open(() => documentsApi.pdf(id)));
const downloadPdf = () => void withReload((id) => download(() => documentsApi.pdf(id, true), fallbackName.value));
</script>

<template>
  <div class="space-y-2" data-test="pdf">
    <div class="flex flex-wrap items-center gap-2">
      <button type="button" class="btn" :disabled="busy" data-test="pdf-open" @click="openPdf">
        {{ isDraft ? t("pdf.preview") : t("pdf.open") }}
      </button>
      <button type="button" class="btn" :disabled="busy" data-test="pdf-download" @click="downloadPdf">
        {{ t("pdf.download") }}
      </button>
      <span v-if="busy" class="text-sm text-gray-500">{{ t("pdf.loading") }}</span>
      <span v-else-if="doc.pdf" class="text-sm text-gray-600 dark:text-gray-400" data-test="pdf-archived">
        {{ t("pdf.archivedAt", { date: formatDate(doc.pdf.renderedAt, locale) }) }}
      </span>
    </div>
    <p v-if="isDraft" class="text-sm text-gray-600 dark:text-gray-400" data-test="pdf-draft-hint">{{ t("pdf.draftHint") }}</p>
    <div v-if="error" role="alert" class="alert-error" data-test="pdf-error">
      <p>{{ error }}</p>
      <ErrorDetail :detail="detail" />
    </div>
  </div>
</template>
