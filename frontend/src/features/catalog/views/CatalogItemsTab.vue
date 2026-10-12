<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import { formatMoney } from "@/lib/format";
import CatalogItemForm from "../components/CatalogItemForm.vue";
import CatalogSearch from "../components/CatalogSearch.vue";
import { useCatalogItemsStore } from "../store";
import type { CatalogItem } from "../types";
import { useSessionStore } from "@/stores/session";

const { t, locale } = useI18n();
const session = useSessionStore();
const store = useCatalogItemsStore();
const { error, run } = useAction();
/** null = form closed, "new" = create, otherwise the item being edited. */
const editing = ref<CatalogItem | "new" | null>(null);

onMounted(() => void run(store.load));

function onDelete(item: CatalogItem) {
  if (!window.confirm(t("catalog.items.confirmDelete", { name: item.name }))) return;
  void run(() => store.remove(item.id));
}
</script>

<template>
  <div class="space-y-4">
    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>

    <CatalogItemForm v-if="editing" :key="editing === 'new' ? 'new' : editing.id" :item="editing === 'new' ? undefined : editing" @done="editing = null" />
    <div v-else class="flex flex-wrap items-center justify-between gap-3">
      <CatalogSearch id="catalog-items-search" :initial="store.q" :placeholder="t('catalog.items.searchPlaceholder')" @search="(q) => run(() => store.search(q))" />
      <button type="button" v-if="session.can('write')" class="btn btn-primary" data-test="add-item" @click="editing = 'new'">{{ t("catalog.items.add") }}</button>
    </div>

    <div class="card overflow-x-auto !p-0">
      <table class="table">
        <thead>
          <tr>
            <th>{{ t("catalog.items.name") }}</th>
            <th class="text-right">{{ t("documents.line.unitPrice") }}</th>
            <th class="hidden text-right sm:table-cell">{{ t("documents.line.vatRate") }}</th>
            <th class="w-0"><span class="sr-only">{{ t("common.actions") }}</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="i in store.items" :key="i.id" :class="{ 'opacity-60': !i.active }" data-test="item-row">
            <td>
              <span class="font-medium">{{ i.name }}</span>
              <span v-if="!i.active" class="badge ml-2 !bg-gray-200 !text-gray-700 dark:!bg-gray-700 dark:!text-gray-200">{{ t("catalog.items.inactive") }}</span>
              <div v-if="i.note" class="text-xs text-gray-500">{{ i.note }}</div>
            </td>
            <td class="text-right whitespace-nowrap tabular-nums">
              {{ formatMoney(i.unitPrice, i.currency, locale) }}<template v-if="i.unit"> / {{ i.unit }}</template>
              <div class="text-xs text-gray-500 sm:hidden">{{ i.vatRate }} %</div>
            </td>
            <td class="hidden text-right sm:table-cell">{{ i.vatRate }} %</td>
            <td class="whitespace-nowrap">
              <div v-if="session.can('write')" class="flex gap-1">
                <button type="button" class="btn btn-sm" @click="editing = i">{{ t("common.edit") }}</button>
                <button type="button" class="btn btn-sm btn-danger" @click="onDelete(i)">{{ t("common.delete") }}</button>
              </div>
            </td>
          </tr>
          <tr v-if="store.loaded && store.items.length === 0">
            <td colspan="4" class="py-8 text-center text-gray-500">{{ store.q ? t("catalog.noResults") : t("catalog.items.empty") }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
