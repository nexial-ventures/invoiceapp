<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { Direction } from "@/api/types";
import FormField from "@/components/form/FormField.vue";
import { useAction } from "@/composables/useAction";
import { documentsApi } from "../api";
import { relatedTargetTypes } from "../docTypes";
import type { DocumentSummary, RelatedDocument } from "../types";

const LIMIT = 50;
const SEARCH_DEBOUNCE_MS = 300;

/** Informational link of a received / imported document to its proforma (or a correction to its original). */
const model = defineModel<string | null>({ required: true });
const props = defineProps<{
  direction: Direction;
  docType: string;
  /** Narrows the choice to the same counterparty once one is picked. */
  contactId: string | null;
  /** The currently linked document, so its label shows even when outside the loaded page. */
  current?: RelatedDocument | null;
  excludeId?: string;
  error?: string;
}>();

const { t } = useI18n();
const { error: loadError, run } = useAction();
const options = ref<DocumentSummary[]>([]);
/** More matches than shown: the search narrows them (number, supplier number, counterparty, VS). */
const more = ref(false);
const query = ref("");
const targets = computed(() => relatedTargetTypes(props.docType));
/** Several target types (invoice + simplified): each option says which one it is. */
const mixed = computed(() => targets.value.length > 1);
/** The only mixed case is a correction's original: invoice or simplified document. */
const labelKey = computed(() => (mixed.value ? "invoiceOrSimplified" : targets.value[0]));
let seq = 0;
let timer: ReturnType<typeof setTimeout> | undefined;

function load() {
  const types = targets.value;
  options.value = [];
  more.value = false;
  if (!types.length) return;
  const mine = ++seq;
  void run(async () => {
    // Drafts cannot be linked (the server rejects them); the newest come first.
    const pages = await Promise.all(
      types.map((docType) =>
        documentsApi.list({
          direction: props.direction,
          docType,
          status: "issued",
          contactId: props.contactId ?? undefined,
          q: query.value,
          limit: LIMIT,
          offset: 0,
        }),
      ),
    );
    if (mine !== seq) return;
    const items = pages.flatMap((p) => p.items).filter((d) => d.id !== props.excludeId);
    // Each page is newest first; merged pages are re-sorted to keep that order.
    options.value = pages.length > 1 ? items.sort((a, b) => b.issueDate.localeCompare(a.issueDate)) : items;
    more.value = pages.some((p) => p.total > p.items.length);
  });
}

watch(() => [targets.value.join(), props.contactId] as const, load, { immediate: true });

function onSearch() {
  clearTimeout(timer);
  timer = setTimeout(load, SEARCH_DEBOUNCE_MS);
}
onBeforeUnmount(() => clearTimeout(timer));

const missingCurrent = computed(() => !!model.value && !options.value.some((d) => d.id === model.value));
const label = (d: { docType: string; number: string | null; supplierNumber?: string | null; customerName?: string | null }) =>
  [mixed.value && t(`documents.docTypes.${d.docType}`), d.number ?? t("documents.draftNumber"), d.supplierNumber, d.customerName].filter(Boolean).join(" · ");
</script>

<template>
  <FormField v-if="targets.length" :label="t(`documents.related.pick.${labelKey}`)" for="related-document" :error="error" :hint="t('documents.related.pickHint')">
    <input
      v-model="query"
      type="search"
      class="input mb-1"
      :placeholder="t('documents.related.search')"
      :aria-label="t('documents.related.search')"
      autocomplete="off"
      data-test="related-search"
      @input="onSearch"
    />
    <select id="related-document" :value="model ?? ''" class="input" :class="{ 'input-error': error }" data-test="related-picker" @change="model = ($event.target as HTMLSelectElement).value || null">
      <option value="">{{ t("documents.related.none") }}</option>
      <option v-if="missingCurrent" :value="model ?? ''">{{ current ? label(current) : model }}</option>
      <option v-for="d in options" :key="d.id" :value="d.id">{{ label(d) }}</option>
    </select>
    <p v-if="more" class="text-xs text-gray-500" data-test="related-more">{{ t("documents.related.more", { n: options.length }) }}</p>
    <p v-if="loadError" role="alert" class="alert-error">{{ loadError }}</p>
  </FormField>
</template>
