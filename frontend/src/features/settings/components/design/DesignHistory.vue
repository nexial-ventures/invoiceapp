<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import ErrorDetail from "@/components/ErrorDetail.vue";
import { useAction } from "@/composables/useAction";
import { formatDate } from "@/features/documents/format";
import { useToastStore } from "@/stores/toast";
import { authorName } from "../../designAuthor";
import { useDesignStore } from "../../stores";
import type { DesignVersion } from "../../types";

/** Activations / restores / resets, newest first; restore a version or fall back to the built-in design. */
const { t, locale } = useI18n();
const store = useDesignStore();
const toast = useToastStore();
const { error, detail, run } = useAction();
const busy = ref(false);

async function act(confirmText: string, action: () => Promise<void>, done: string): Promise<void> {
  if (busy.value || !window.confirm(confirmText)) return;
  busy.value = true;
  try {
    if (await run(action)) toast.show(done);
  } finally {
    busy.value = false;
  }
}

const restore = (v: DesignVersion) =>
  act(t("design.history.confirmRestore", { number: v.number }), () => store.restore(v.id), t("design.history.restored", { number: v.number }));
const reset = () => act(t("design.history.confirmReset"), store.reset, t("design.history.resetDone"));
</script>

<template>
  <section class="card space-y-3" data-test="design-history">
    <div class="flex flex-wrap items-start justify-between gap-2">
      <div class="space-y-1">
        <h2 class="font-semibold">{{ t("design.history.title") }}</h2>
        <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("design.history.hint") }}</p>
      </div>
      <button type="button" class="btn btn-danger" :disabled="busy" data-test="design-reset" @click="reset">
        {{ t("design.history.reset") }}
      </button>
    </div>

    <div class="overflow-x-auto">
      <table class="table">
        <thead>
          <tr>
            <th>{{ t("design.history.number") }}</th>
            <th>{{ t("design.history.kind") }}</th>
            <th>{{ t("design.history.author") }}</th>
            <th>{{ t("design.history.time") }}</th>
            <th class="text-right">{{ t("design.history.files") }}</th>
            <th><span class="sr-only">{{ t("common.actions") }}</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="v in store.versions" :key="v.id" data-test="design-version">
            <td class="whitespace-nowrap">
              {{ v.number }}
              <span v-if="v.current" class="badge ml-1" data-test="design-version-current">{{ t("design.history.current") }}</span>
            </td>
            <td>{{ t(`design.history.kinds.${v.kind}`) }}</td>
            <td :title="v.createdBy?.email" :class="{ 'text-gray-500 italic': !v.createdBy }">
              {{ authorName(v.createdBy) ?? t("design.deletedUser") }}
            </td>
            <td class="whitespace-nowrap">{{ formatDate(v.createdAt, locale) }}</td>
            <td class="text-right">{{ v.files.length }}</td>
            <td class="text-right">
              <button
                v-if="!v.current"
                type="button"
                class="btn btn-sm"
                :disabled="busy"
                data-test="design-version-restore"
                @click="restore(v)"
              >
                {{ t("design.history.restore") }}
              </button>
            </td>
          </tr>
          <tr v-if="store.versions.length === 0">
            <td colspan="6" class="py-6 text-center text-gray-500">{{ t("design.history.empty") }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <div v-if="error" role="alert" class="alert-error" data-test="design-history-error">
      <p>{{ error }}</p>
      <ErrorDetail :detail="detail" open />
    </div>
  </section>
</template>
