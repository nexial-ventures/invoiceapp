<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { DocLocale } from "@/api/types";
import ErrorDetail from "@/components/ErrorDetail.vue";
import { useAction } from "@/composables/useAction";
import { usePdf } from "@/composables/usePdf";
import { formatBytes } from "@/lib/bytes";
import { useToastStore } from "@/stores/toast";
import { formatDate } from "@/features/documents/format";
import { pdfApi } from "../../api";
import { authorName } from "../../designAuthor";
import { useDesignStore } from "../../stores";
import DesignUploadForm from "./DesignUploadForm.vue";

/** The draft: upload, pending changes, preview, activate, discard. */
const { t, locale } = useI18n();
const store = useDesignStore();
const toast = useToastStore();
const { error, detail, run } = useAction();
const pdf = usePdf();
const busy = ref(false);
const uploadForm = ref<InstanceType<typeof DesignUploadForm> | null>(null);

const draft = computed(() => store.design?.draft ?? null);
const empty = computed(() => !draft.value || (draft.value.files.length === 0 && draft.value.deleted.length === 0));

async function act(action: () => Promise<void>, done?: string): Promise<void> {
  if (busy.value) return;
  busy.value = true;
  try {
    if ((await run(action)) && done) toast.show(done);
  } finally {
    busy.value = false;
  }
}

/** A draft file is dropped; a live custom file is marked for deletion. */
const remove = (path: string) => act(() => store.removeDraftFile(path));
const activate = () => act(store.activate, t("design.draft.activated"));
function discard() {
  if (!window.confirm(t("design.draft.confirmDiscard"))) return;
  void act(store.discard, t("design.draft.discarded"));
}

const preview = (l: DocLocale) => void pdf.open(() => pdfApi.draftPreview(l));
const downloadDraft = (path: string) =>
  void pdf.download(() => pdfApi.designFile(path, "draft"), path.slice(path.lastIndexOf("/") + 1));

const previews: { locale: DocLocale; label: string }[] = [
  { locale: "cs", label: "design.draft.previewCs" },
  { locale: "en", label: "design.draft.previewEn" },
];

defineExpose({ pick: (path: string) => uploadForm.value?.pick(path), remove });
</script>

<template>
  <section class="card space-y-4" data-test="design-draft">
    <div class="space-y-1">
      <h2 class="font-semibold">{{ t("design.draft.title") }}</h2>
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("design.draft.hint") }}</p>
    </div>

    <DesignUploadForm ref="uploadForm" />

    <div class="space-y-2">
      <div class="flex flex-wrap items-baseline justify-between gap-2">
        <h3 class="text-sm font-medium">{{ t("design.draft.changes") }}</h3>
        <span v-if="draft?.updatedAt && !empty" class="text-xs text-gray-500" data-test="design-draft-updated">
          {{ t("design.draft.updated", { date: formatDate(draft.updatedAt, locale), by: authorName(draft.updatedBy) ?? t("design.deletedUser") }) }}
        </span>
      </div>
      <p v-if="empty" class="text-sm text-gray-500" data-test="design-draft-empty">{{ t("design.draft.empty") }}</p>
      <ul v-else class="divide-y divide-gray-100 text-sm dark:divide-gray-800">
        <li v-for="f in draft?.files" :key="f.path" class="flex flex-wrap items-center gap-2 py-1.5" data-test="design-draft-file">
          <span class="font-mono">{{ f.path }}</span>
          <span class="badge" :data-test="`change-${f.change}`">{{ t(`design.draft.kinds.${f.change}`) }}</span>
          <span class="text-gray-500">{{ formatBytes(f.size, locale) }}</span>
          <span class="ml-auto flex gap-1">
            <button type="button" class="btn btn-sm" :disabled="pdf.busy.value" data-test="design-draft-download" @click="downloadDraft(f.path)">
              {{ t("design.download") }}
            </button>
            <button type="button" class="btn btn-sm" :disabled="busy" data-test="design-draft-remove" @click="remove(f.path)">
              {{ t("design.draft.remove") }}
            </button>
          </span>
        </li>
        <li v-for="p in draft?.deleted" :key="p" class="flex flex-wrap items-center gap-2 py-1.5" data-test="design-draft-deleted">
          <span class="font-mono line-through">{{ p }}</span>
          <span class="badge !bg-red-100 !text-red-800 dark:!bg-red-900/50 dark:!text-red-300">{{ t("design.draft.kinds.deleted") }}</span>
        </li>
      </ul>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <button
        v-for="p in previews"
        :key="p.locale"
        type="button"
        class="btn"
        :disabled="pdf.busy.value"
        :data-test="`design-draft-preview-${p.locale}`"
        @click="preview(p.locale)"
      >
        {{ t(p.label) }}
      </button>
      <button type="button" class="btn btn-primary" :disabled="busy || empty" data-test="design-activate" @click="activate">
        {{ t("design.draft.activate") }}
      </button>
      <button type="button" class="btn btn-danger" :disabled="busy || empty" data-test="design-discard" @click="discard">
        {{ t("design.draft.discard") }}
      </button>
      <span v-if="pdf.busy.value" class="text-sm text-gray-500">{{ t("pdf.loading") }}</span>
    </div>

    <div v-if="error" role="alert" class="alert-error" data-test="design-draft-error">
      <p>{{ error }}</p>
      <ErrorDetail :detail="detail" open />
    </div>
    <div v-if="pdf.error.value" role="alert" class="alert-error" data-test="design-draft-preview-error">
      <p>{{ pdf.error.value }}</p>
      <ErrorDetail :detail="pdf.detail.value" open />
    </div>
  </section>
</template>
