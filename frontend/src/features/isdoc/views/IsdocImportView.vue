<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import ImportConfirmBar from "@/features/imports/components/ImportConfirmBar.vue";
import ImportDropZone from "@/features/imports/components/ImportDropZone.vue";
import ImportResultTable from "@/features/imports/components/ImportResultTable.vue";
import { MAX_UPLOAD_BYTES, totalBytes } from "@/features/imports/upload";
import { useImportPage } from "@/features/imports/useImportPage";
import { formatBytes } from "@/lib/bytes";
import { isApiError } from "@/lib/formErrors";
import IsdocBatchOptions from "../components/IsdocBatchOptions.vue";
import IsdocPreviewTable from "../components/IsdocPreviewTable.vue";
import { useIsdocImportStore } from "../store";
import { ACCEPT_ATTR, entryErrorKey, partitionFiles, uploadErrorKey } from "../upload";

const { t, locale } = useI18n();
const store = useIsdocImportStore();
const { busy, error, backTo, act } = useImportPage((err) => {
  if (isApiError(err, 422, "quota_exceeded")) return t("isdoc.upload.quotaExceeded", { detail: err.detail ?? "" });
  const key = uploadErrorKey(err);
  return key ? t(key) : null;
});

// Every visit starts a fresh import; the results of an earlier one are not kept around.
store.reset();

const ignored = ref<string[]>([]);

const resultError = (code: string) => t(entryErrorKey(code), { code, number: "" });

function onFiles(picked: File[]) {
  const { files, ignored: rest } = partitionFiles(picked);
  ignored.value = rest;
  error.value = null;
  if (files.length === 0) {
    store.reset();
    error.value = t("isdoc.upload.noFiles");
    return;
  }
  if (totalBytes(files) > MAX_UPLOAD_BYTES) {
    store.reset();
    error.value = t("isdoc.upload.tooLarge");
    return;
  }
  void act(() => store.preview(files));
}

const confirm = () => void act(store.confirm);

function restart() {
  store.reset();
  ignored.value = [];
  error.value = null;
}
</script>

<template>
  <section class="space-y-4">
    <RouterLink :to="backTo" class="text-sm text-blue-600 hover:underline dark:text-blue-400" data-test="import-back">
      ← {{ t("imports.page.back") }}
    </RouterLink>
    <h1 class="text-2xl font-semibold">{{ t("isdoc.import.title") }}</h1>

    <template v-if="!store.results">
      <ImportDropZone
        :accept="ACCEPT_ATTR"
        multiple
        :disabled="busy"
        :hint="t('isdoc.import.dropHint')"
        :pick="t('isdoc.import.pick')"
        :limits="t('isdoc.import.limits')"
        @files="onFiles"
      />
      <p v-if="store.files.length" class="text-sm text-gray-600 dark:text-gray-400" data-test="isdoc-files">
        {{ t("isdoc.import.files", { names: store.files.map((f) => f.name).join(", "), size: formatBytes(totalBytes(store.files), locale) }) }}
      </p>
      <p v-if="ignored.length" class="text-sm text-amber-700 dark:text-amber-400" data-test="isdoc-ignored">
        {{ t("isdoc.import.ignored", { names: ignored.join(", ") }) }}
      </p>
    </template>

    <p v-if="error" role="alert" class="alert-error" data-test="import-error-alert">{{ error }}</p>
    <p v-if="busy" class="text-sm text-gray-500">{{ store.entries ? t("imports.page.importing") : t("imports.page.reading") }}</p>

    <template v-if="store.results">
      <ImportResultTable :results="store.results" :entries="store.entries ?? []" :error-text="resultError" />
      <button type="button" class="btn" data-test="import-restart" @click="restart">{{ t("isdoc.result.again") }}</button>
    </template>

    <template v-else-if="store.entries">
      <IsdocPreviewTable />
      <IsdocBatchOptions />
      <ImportConfirmBar :count="store.selectedCount" :total="store.entries.length" :busy="busy" @confirm="confirm" />
    </template>
  </section>
</template>
