<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { DocLocale, FieldErrors } from "@/api/types";
import FormField from "@/components/form/FormField.vue";
import { LOCALES } from "@/i18n";
import { PAYMENT_METHODS, ROUNDING_MODES } from "@/features/documents/types";
import type { CompanyDraft } from "../companyForm";

/** Settings → Firma, "Výchozí hodnoty dokladů": what a new document starts from. */
const model = defineModel<CompanyDraft>({ required: true });
const props = defineProps<{ errors: FieldErrors }>();

const { t } = useI18n();
const noteLocale = ref<DocLocale>("cs");
const noteError = (kind: "Header" | "Footer", code: DocLocale) => props.errors[`default${kind}Note.${code}`];
/** Flags a tab whose (hidden) notes carry an error. */
const tabHasError = (code: DocLocale) => !!(noteError("Header", code) || noteError("Footer", code));
// A 422 on the hidden language's note would go unseen: show that tab.
watch(
  () => props.errors,
  () => {
    const failing = LOCALES.find(tabHasError);
    if (failing && !tabHasError(noteLocale.value)) noteLocale.value = failing;
  },
);
const cls = (field: string) => ({ "input-error": props.errors[field] });
</script>

<template>
  <section class="card space-y-4" data-test="document-defaults">
    <div>
      <h2 class="font-semibold">{{ t("settings.company.defaults.title") }}</h2>
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("settings.company.defaults.hint") }}</p>
    </div>
    <div class="grid gap-4 sm:grid-cols-2">
      <FormField :label="t('settings.company.defaults.paymentMethod')" for="company-defaultPaymentMethod" :error="errors.defaultPaymentMethod">
        <select id="company-defaultPaymentMethod" v-model="model.defaultPaymentMethod" class="input" :class="cls('defaultPaymentMethod')">
          <option v-for="m in PAYMENT_METHODS" :key="m" :value="m">{{ t(`documents.paymentMethod.${m}`) }}</option>
        </select>
      </FormField>
      <FormField
        :label="t('settings.company.defaults.currency')"
        for="company-defaultCurrency"
        :error="errors.defaultCurrency"
        :hint="t('settings.company.defaults.currencyHint')"
      >
        <input id="company-defaultCurrency" v-model="model.defaultCurrency" maxlength="3" class="input uppercase" :class="cls('defaultCurrency')" autocomplete="off" />
      </FormField>
      <FormField :label="t('settings.company.defaults.roundingCash')" for="company-defaultRoundingCash" :error="errors.defaultRoundingCash">
        <select id="company-defaultRoundingCash" v-model="model.defaultRoundingCash" class="input" :class="cls('defaultRoundingCash')">
          <option v-for="m in ROUNDING_MODES" :key="m" :value="m">{{ t(`documents.roundingMode.${m}`) }}</option>
        </select>
      </FormField>
      <FormField :label="t('settings.company.defaults.roundingOther')" for="company-defaultRoundingOther" :error="errors.defaultRoundingOther">
        <select id="company-defaultRoundingOther" v-model="model.defaultRoundingOther" class="input" :class="cls('defaultRoundingOther')">
          <option v-for="m in ROUNDING_MODES" :key="m" :value="m">{{ t(`documents.roundingMode.${m}`) }}</option>
        </select>
      </FormField>
    </div>

    <div class="space-y-3">
      <div role="tablist" class="flex gap-1 border-b border-gray-200 dark:border-gray-800">
        <button
          v-for="code in LOCALES"
          :key="code"
          type="button"
          role="tab"
          :aria-selected="noteLocale === code"
          class="-mb-px border-b-2 px-3 py-1.5 text-sm"
          :class="[
            noteLocale === code ? 'border-blue-600 font-medium' : 'border-transparent text-gray-600 dark:text-gray-400',
            { 'text-red-600 dark:text-red-400': tabHasError(code) },
          ]"
          :data-test="`notes-tab-${code}`"
          @click="noteLocale = code"
        >
          {{ t(`locale.names.${code}`) }}<span v-if="tabHasError(code)" aria-hidden="true"> •</span>
        </button>
      </div>
      <div class="grid gap-4 sm:grid-cols-2">
        <FormField :label="t('documents.fields.headerNote')" :for="`company-defaultHeaderNote-${noteLocale}`" :error="noteError('Header', noteLocale)">
          <textarea
            :id="`company-defaultHeaderNote-${noteLocale}`"
            v-model="model.defaultHeaderNote[noteLocale]"
            rows="3"
            maxlength="2000"
            class="input"
            :class="{ 'input-error': noteError('Header', noteLocale) }"
          />
        </FormField>
        <FormField :label="t('documents.fields.footerNote')" :for="`company-defaultFooterNote-${noteLocale}`" :error="noteError('Footer', noteLocale)">
          <textarea
            :id="`company-defaultFooterNote-${noteLocale}`"
            v-model="model.defaultFooterNote[noteLocale]"
            rows="3"
            maxlength="2000"
            class="input"
            :class="{ 'input-error': noteError('Footer', noteLocale) }"
          />
        </FormField>
      </div>
    </div>
  </section>
</template>
