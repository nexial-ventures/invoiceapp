<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useCategoriesStore } from "@/features/settings/stores";
import { useIsdocImportStore } from "../store";

/** Options applied to every imported entry; category and VAT deductible only to received ones. */
const { t } = useI18n();
const store = useIsdocImportStore();
const categoriesStore = useCategoriesStore();

// The category is optional: without the list the import still works.
categoriesStore.ensure().catch(() => {});
const categories = computed(() => categoriesStore.items.filter((c) => c.kind === "expense" && c.active));
</script>

<template>
  <fieldset class="card space-y-3" data-test="isdoc-options">
    <legend class="sr-only">{{ t("isdoc.options.title") }}</legend>
    <label class="flex items-center gap-2 text-sm">
      <input v-model="store.options.markPaid" type="checkbox" data-test="isdoc-mark-paid" />
      {{ t("isdoc.options.markPaid") }}
    </label>
    <p class="text-xs text-gray-500 dark:text-gray-400">{{ t("isdoc.options.markPaidHint") }}</p>
    <div v-if="store.receivedSelected" class="flex flex-wrap items-end gap-4 border-t border-gray-100 pt-3 dark:border-gray-800" data-test="isdoc-received-options">
      <label class="block text-sm">
        <span class="mb-1 block font-medium">{{ t("isdoc.options.category") }}</span>
        <select v-model="store.options.categoryId" class="input" data-test="isdoc-category">
          <option :value="null">{{ t("isdoc.options.noCategory") }}</option>
          <option v-for="c in categories" :key="c.id" :value="c.id">{{ c.name }}</option>
        </select>
      </label>
      <label class="flex items-center gap-2 py-2 text-sm">
        <input v-model="store.options.vatDeductible" type="checkbox" data-test="isdoc-vat-deductible" />
        {{ t("received.fields.vatDeductible") }}
      </label>
      <p class="w-full text-xs text-gray-500 dark:text-gray-400">{{ t("isdoc.options.receivedHint") }}</p>
    </div>
  </fieldset>
</template>
