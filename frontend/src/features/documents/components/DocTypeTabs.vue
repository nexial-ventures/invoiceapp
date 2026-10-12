<script setup lang="ts">
import { useI18n } from "vue-i18n";
import type { Direction } from "@/api/types";
import { listLocation } from "../routes";
import { LIST_DOC_TYPES, type ListDocType } from "../store";

/** Doc-type tabs of a document list (`?type=`); `labelPrefix` + type gives each tab's label. */
defineProps<{ current: ListDocType; direction: Direction; labelPrefix: string }>();
const { t } = useI18n();
</script>

<template>
  <nav :aria-label="t('documents.list.tabs')" class="-mx-4 overflow-x-auto [scrollbar-width:none] border-b border-gray-200 px-4 sm:mx-0 sm:px-0 dark:border-gray-800">
    <ul class="flex gap-1 whitespace-nowrap">
      <li v-for="type in LIST_DOC_TYPES" :key="type">
        <RouterLink
          :to="listLocation(type, direction)"
          class="inline-block border-b-2 border-transparent px-3 py-2 text-sm text-gray-600 hover:text-gray-900 dark:text-gray-400 dark:hover:text-gray-100"
          :class="{ '!border-blue-600 !text-gray-900 font-medium dark:!text-gray-100': type === current }"
          :aria-current="type === current ? 'page' : undefined"
          :data-test="`tab-${type}`"
        >
          {{ t(`${labelPrefix}.${type}`) }}
        </RouterLink>
      </li>
    </ul>
  </nav>
</template>
