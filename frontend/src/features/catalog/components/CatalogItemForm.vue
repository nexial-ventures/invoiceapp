<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { defaultVatRate } from "@/features/documents/form";
import { useVatRatesStore } from "@/features/settings/stores";
import { toItemDraft, toItemInput, validateItem } from "../form";
import { useCatalogItemsStore } from "../store";
import type { CatalogItem } from "../types";

const props = defineProps<{ item?: CatalogItem }>();
const emit = defineEmits<{ done: [] }>();

const { t } = useI18n();
const store = useCatalogItemsStore();
const vatRates = useVatRatesStore();
const { fieldErrors, error, submitting, submit } = useFormSubmit();

const draft = ref(toItemDraft(props.item, ""));

onMounted(async () => {
  await vatRates.ensure().catch(() => {});
  if (!draft.value.vatRate) draft.value.vatRate = defaultVatRate(vatRates.items, "standard");
});

/** Active rates, plus the item's own rate should it have been retired since. */
const rateOptions = computed(() => {
  const active = vatRates.items.filter((r) => r.active).map((r) => r.rate);
  const current = draft.value.vatRate;
  return current && !active.includes(current) ? [...active, current] : active;
});

const cls = (field: string) => ({ "input-error": fieldErrors.value[field] });

async function onSubmit() {
  const ok = await submit(
    () => validateItem(draft.value),
    () => store.save(toItemInput(draft.value), props.item?.id),
  );
  if (ok) emit("done");
}
</script>

<template>
  <form class="card space-y-4" novalidate data-test="item-form" @submit.prevent="onSubmit">
    <h2 class="font-semibold">{{ item ? t("catalog.items.edit") : t("catalog.items.add") }}</h2>
    <div class="grid gap-4 sm:grid-cols-6">
      <FormField class="sm:col-span-4" :label="t('catalog.items.name')" for="item-name" :error="fieldErrors.name">
        <input id="item-name" v-model="draft.name" maxlength="200" class="input" :class="cls('name')" />
      </FormField>
      <FormField class="sm:col-span-2" :label="t('documents.line.unit')" for="item-unit" :error="fieldErrors.unit">
        <input id="item-unit" v-model="draft.unit" maxlength="20" class="input" :class="cls('unit')" />
      </FormField>
      <FormField class="sm:col-span-2" :label="t('documents.line.unitPrice')" for="item-unitPrice" :error="fieldErrors.unitPrice">
        <input id="item-unitPrice" v-model="draft.unitPrice" inputmode="decimal" class="input" :class="cls('unitPrice')" />
      </FormField>
      <FormField class="sm:col-span-2" :label="t('documents.fields.currency')" for="item-currency" :error="fieldErrors.currency">
        <input id="item-currency" v-model="draft.currency" maxlength="3" class="input uppercase" :class="cls('currency')" autocomplete="off" />
      </FormField>
      <FormField class="sm:col-span-2" :label="t('documents.line.vatRate')" for="item-vatRate" :error="fieldErrors.vatRate">
        <select id="item-vatRate" v-model="draft.vatRate" class="input" :class="cls('vatRate')">
          <option v-for="r in rateOptions" :key="r" :value="r">{{ r }} %</option>
        </select>
      </FormField>
      <FormField class="sm:col-span-6" :label="t('catalog.items.note')" for="item-note" :error="fieldErrors.note">
        <textarea id="item-note" v-model="draft.note" rows="2" maxlength="2000" class="input" />
      </FormField>
    </div>
    <label class="flex items-center gap-2 text-sm">
      <input id="item-active" v-model="draft.active" type="checkbox" class="size-4 rounded" />
      {{ t("catalog.items.active") }}
    </label>
    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    <div class="flex gap-2">
      <button type="submit" class="btn btn-primary" :disabled="submitting" data-test="save-item">
        {{ submitting ? t("common.saving") : t("common.save") }}
      </button>
      <button type="button" class="btn" @click="emit('done')">{{ t("common.cancel") }}</button>
    </div>
  </form>
</template>
