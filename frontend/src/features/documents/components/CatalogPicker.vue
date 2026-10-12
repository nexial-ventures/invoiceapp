<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import { catalogGroupsApi, catalogItemsApi } from "@/features/catalog/api";
import type { CatalogGroup, CatalogItem } from "@/features/catalog/types";
import { formatMoney } from "@/lib/format";

const SEARCH_DEBOUNCE_MS = 300;

defineProps<{ currency: string }>();
const emit = defineEmits<{ item: [item: CatalogItem]; group: [group: CatalogGroup] }>();

const { t, locale } = useI18n();
const { error, run } = useAction();

const open = ref(false);
const query = ref("");
const items = ref<CatalogItem[]>([]);
const groups = ref<CatalogGroup[]>([]);
const loaded = ref(false);
let timer: ReturnType<typeof setTimeout> | undefined;
let seq = 0;

async function search() {
  const mine = ++seq;
  const [foundItems, foundGroups] = await Promise.all([catalogItemsApi.list(query.value, true), catalogGroupsApi.list(query.value)]);
  if (mine !== seq) return;
  items.value = foundItems;
  groups.value = foundGroups;
  loaded.value = true;
}

function toggle() {
  open.value = !open.value;
  if (open.value) void run(search);
}

function onInput() {
  clearTimeout(timer);
  timer = setTimeout(() => void run(search), SEARCH_DEBOUNCE_MS);
}

function pickItem(item: CatalogItem) {
  open.value = false;
  emit("item", item);
}

function pickGroup(group: CatalogGroup) {
  open.value = false;
  emit("group", group);
}

onBeforeUnmount(() => clearTimeout(timer));
</script>

<template>
  <!-- `contents`: the button sits in the parent's button row, the panel wraps below it. -->
  <div class="contents">
    <button type="button" class="btn btn-sm" :aria-expanded="open" data-test="add-catalog" @click="toggle">+ {{ t("documents.catalog.add") }}</button>
    <div v-if="open" class="basis-full space-y-2 rounded-md border border-gray-200 p-3 dark:border-gray-700" data-test="catalog-picker">
      <label for="catalog-search" class="sr-only">{{ t("common.search") }}</label>
      <input id="catalog-search" v-model="query" type="search" class="input" :placeholder="t('documents.catalog.searchPlaceholder')" autocomplete="off" @input="onInput" />
      <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
      <div v-if="groups.length" class="space-y-1">
        <h3 class="text-xs font-semibold tracking-wide text-gray-500 uppercase">{{ t("documents.catalog.groups") }}</h3>
        <ul class="max-h-48 overflow-y-auto rounded-md border border-gray-200 dark:border-gray-700">
          <li v-for="g in groups" :key="g.id">
            <button type="button" class="w-full px-3 py-2 text-left text-sm hover:bg-gray-100 dark:hover:bg-gray-800" data-test="catalog-group" @click="pickGroup(g)">
              <span class="font-medium">{{ g.name }}</span>
              <span class="ml-2 text-gray-500">{{ t("documents.catalog.memberCount", { n: g.members.length }) }}</span>
            </button>
          </li>
        </ul>
      </div>
      <div v-if="items.length" class="space-y-1">
        <h3 class="text-xs font-semibold tracking-wide text-gray-500 uppercase">{{ t("documents.catalog.items") }}</h3>
        <ul class="max-h-60 overflow-y-auto rounded-md border border-gray-200 dark:border-gray-700">
          <li v-for="i in items" :key="i.id">
            <button type="button" class="flex w-full flex-wrap justify-between gap-x-3 px-3 py-2 text-left text-sm hover:bg-gray-100 dark:hover:bg-gray-800" data-test="catalog-item" @click="pickItem(i)">
              <span class="font-medium">{{ i.name }}</span>
              <span class="tabular-nums" :class="i.currency === currency ? 'text-gray-500' : 'text-amber-700 dark:text-amber-400'">
                {{ formatMoney(i.unitPrice, i.currency, locale) }}<template v-if="i.unit"> / {{ i.unit }}</template> · {{ i.vatRate }} %
              </span>
            </button>
          </li>
        </ul>
      </div>
      <p v-if="loaded && !items.length && !groups.length" class="text-xs text-gray-500">{{ t("documents.catalog.noResults") }}</p>
    </div>
  </div>
</template>
