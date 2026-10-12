<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import { formatNumber } from "@/lib/format";
import CatalogGroupForm from "../components/CatalogGroupForm.vue";
import CatalogSearch from "../components/CatalogSearch.vue";
import { useCatalogGroupsStore } from "../store";
import type { CatalogGroup } from "../types";
import { useSessionStore } from "@/stores/session";

const { t, locale } = useI18n();
const session = useSessionStore();
const store = useCatalogGroupsStore();
const { error, run } = useAction();
/** null = form closed, "new" = create, otherwise the group being edited. */
const editing = ref<CatalogGroup | "new" | null>(null);

onMounted(() => void run(store.load));

function onDelete(group: CatalogGroup) {
  if (!window.confirm(t("catalog.groups.confirmDelete", { name: group.name }))) return;
  void run(() => store.remove(group.id));
}

const summary = (g: CatalogGroup) =>
  [...g.members]
    .sort((a, b) => a.position - b.position)
    .map((m) => `${formatNumber(m.quantity, locale.value)}× ${m.item.name}`)
    .join(", ");
</script>

<template>
  <div class="space-y-4">
    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>

    <CatalogGroupForm v-if="editing" :key="editing === 'new' ? 'new' : editing.id" :group="editing === 'new' ? undefined : editing" @done="editing = null" />
    <div v-else class="flex flex-wrap items-center justify-between gap-3">
      <CatalogSearch id="catalog-groups-search" :initial="store.q" :placeholder="t('catalog.groups.searchPlaceholder')" @search="(q) => run(() => store.search(q))" />
      <button type="button" v-if="session.can('write')" class="btn btn-primary" data-test="add-group" @click="editing = 'new'">{{ t("catalog.groups.add") }}</button>
    </div>

    <div class="card overflow-x-auto !p-0">
      <table class="table">
        <thead>
          <tr>
            <th>{{ t("catalog.groups.name") }}</th>
            <th class="hidden sm:table-cell">{{ t("catalog.groups.members") }}</th>
            <th class="w-0"><span class="sr-only">{{ t("common.actions") }}</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="g in store.items" :key="g.id" data-test="group-row">
            <td>
              <span class="font-medium">{{ g.name }}</span>
              <span v-if="g.collapse" class="badge ml-2">{{ t("documents.line.collapsed") }}</span>
              <div class="text-xs text-gray-500 sm:hidden">{{ summary(g) }}</div>
            </td>
            <td class="hidden text-sm text-gray-600 sm:table-cell dark:text-gray-400">{{ summary(g) }}</td>
            <td class="whitespace-nowrap">
              <div v-if="session.can('write')" class="flex gap-1">
                <button type="button" class="btn btn-sm" @click="editing = g">{{ t("common.edit") }}</button>
                <button type="button" class="btn btn-sm btn-danger" @click="onDelete(g)">{{ t("common.delete") }}</button>
              </div>
            </td>
          </tr>
          <tr v-if="store.loaded && store.items.length === 0">
            <td colspan="3" class="py-8 text-center text-gray-500">{{ store.q ? t("catalog.noResults") : t("catalog.groups.empty") }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
