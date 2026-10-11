<script setup lang="ts">
import { useI18n } from "vue-i18n";
import ErrorDetail from "@/components/ErrorDetail.vue";
import { usePdf } from "@/composables/usePdf";
import { formatBytes } from "@/lib/bytes";
import { pdfApi } from "../../api";
import type { DesignFile } from "../../types";

/** The live set: download (as a starting point), replace / delete through the draft. */
defineProps<{ files: DesignFile[] }>();
const emit = defineEmits<{ replace: [path: string]; delete: [path: string] }>();

const { t, locale } = useI18n();
const file = usePdf();

const basename = (path: string) => path.slice(path.lastIndexOf("/") + 1);
const download = (f: DesignFile) =>
  void file.download(() => pdfApi.designFile(f.path, f.source === "custom" ? "live" : "default"), basename(f.path));
</script>

<template>
  <div class="card overflow-x-auto !p-0">
    <table class="table">
      <caption class="px-3 pt-3 text-left text-sm font-medium">{{ t("pdf.design.files") }}</caption>
      <thead>
        <tr>
          <th>{{ t("pdf.design.path") }}</th>
          <th>{{ t("pdf.design.source") }}</th>
          <th class="text-right">{{ t("pdf.design.size") }}</th>
          <th><span class="sr-only">{{ t("common.actions") }}</span></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="f in files" :key="f.path" data-test="design-file">
          <td class="font-mono">{{ f.path }}</td>
          <td>
            <span
              class="badge"
              :class="{ '!bg-gray-100 !text-gray-700 dark:!bg-gray-800 dark:!text-gray-300': f.source === 'default' }"
              :data-test="`source-${f.source}`"
            >
              {{ t(`pdf.design.sources.${f.source}`) }}
            </span>
          </td>
          <td class="text-right whitespace-nowrap">{{ formatBytes(f.size, locale) }}</td>
          <td class="whitespace-nowrap text-right">
            <div class="flex justify-end gap-1">
              <button type="button" class="btn btn-sm" :disabled="file.busy.value" data-test="design-file-download" @click="download(f)">
                {{ t("design.download") }}
              </button>
              <button type="button" class="btn btn-sm" data-test="design-file-replace" @click="emit('replace', f.path)">
                {{ t("design.replace") }}
              </button>
              <button
                v-if="f.source === 'custom'"
                type="button"
                class="btn btn-sm btn-danger"
                data-test="design-file-delete"
                @click="emit('delete', f.path)"
              >
                {{ t("design.delete") }}
              </button>
            </div>
          </td>
        </tr>
        <tr v-if="files.length === 0">
          <td colspan="4" class="py-8 text-center text-gray-500">{{ t("pdf.design.empty") }}</td>
        </tr>
      </tbody>
    </table>
    <div v-if="file.error.value" role="alert" class="alert-error m-3" data-test="design-file-error">
      <p>{{ file.error.value }}</p>
      <ErrorDetail :detail="file.detail.value" />
    </div>
  </div>
</template>
