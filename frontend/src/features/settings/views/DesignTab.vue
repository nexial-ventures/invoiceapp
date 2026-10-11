<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { DocLocale } from "@/api/types";
import ErrorDetail from "@/components/ErrorDetail.vue";
import { useAction } from "@/composables/useAction";
import { usePdf } from "@/composables/usePdf";
import DesignDraftPanel from "../components/design/DesignDraftPanel.vue";
import DesignFilesTable from "../components/design/DesignFilesTable.vue";
import DesignHistory from "../components/design/DesignHistory.vue";
import DesignQuota from "../components/design/DesignQuota.vue";
import { pdfApi } from "../api";
import { useDesignStore } from "../stores";

const { t } = useI18n();
const store = useDesignStore();
const { error: loadError, run } = useAction();
const { busy, error: previewError, detail, open } = usePdf();

onMounted(() => void run(store.load));

const draftPanel = ref<InstanceType<typeof DesignDraftPanel> | null>(null);

const preview = (l: DocLocale) => open(() => pdfApi.preview(l));

const previews: { locale: DocLocale; label: string }[] = [
  { locale: "cs", label: "pdf.design.previewCs" },
  { locale: "en", label: "pdf.design.previewEn" },
];
</script>

<template>
  <div class="space-y-4">
    <p v-if="loadError" role="alert" class="alert-error">{{ loadError }}</p>

    <section class="card space-y-3">
      <p class="text-sm text-gray-700 dark:text-gray-300" data-test="design-help">{{ t("pdf.design.help") }}</p>
      <dl v-if="store.design" class="flex flex-wrap gap-x-2 text-sm">
        <dt class="text-gray-600 dark:text-gray-400">{{ t("pdf.design.storage") }}:</dt>
        <dd data-test="design-storage">{{ t(`pdf.design.storages.${store.design.storage}`) }}</dd>
      </dl>
      <div class="flex flex-wrap items-center gap-2">
        <button
          v-for="p in previews"
          :key="p.locale"
          type="button"
          class="btn"
          :disabled="busy"
          :data-test="`design-preview-${p.locale}`"
          @click="preview(p.locale)"
        >
          {{ t(p.label) }}
        </button>
        <span class="text-sm text-gray-500">{{ busy ? t("pdf.loading") : t("pdf.design.previewHint") }}</span>
      </div>
      <div v-if="previewError" role="alert" class="alert-error" data-test="design-preview-error">
        <p>{{ previewError }}</p>
        <ErrorDetail :detail="detail" open />
      </div>
    </section>

    <template v-if="store.design">
      <DesignQuota :quota="store.design.quota" class="card" />
      <DesignFilesTable
        :files="store.design.files"
        @replace="(path) => draftPanel?.pick(path)"
        @delete="(path) => draftPanel?.remove(path)"
      />
      <DesignDraftPanel ref="draftPanel" />
      <DesignHistory />
    </template>
  </div>
</template>
