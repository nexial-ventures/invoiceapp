<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { type DocLocale, type FieldErrors, PAYMENT_METHODS, ROUNDING_MODES, type PaymentMethod, type RoundingMode } from "@/api/types";
import FormField from "@/components/form/FormField.vue";
import { LOCALES } from "@/i18n";
import type { BankAccount, Company } from "@/features/settings/types";
import { changeLocale, changePaymentMethod, changeRounding } from "../defaults";
import { isCorrection } from "../docTypes";
import { changeCurrency, type DocumentDraft } from "../form";
import { formatNumber } from "@/lib/format";
import { VAT_MODES, type ExchangeRate } from "../types";
import IndicativeRateNote from "./IndicativeRateNote.vue";

const model = defineModel<DocumentDraft>({ required: true });
const props = defineProps<{
  errors: FieldErrors;
  bankAccounts: BankAccount[];
  /** Source of the defaults re-applied on payment method / locale changes. */
  company: Company;
  /** Indicative ČNB rate for the tax point date (preview only, never saved). */
  indicativeRate: ExchangeRate | null;
  indicativeError: unknown;
  indicativeLoading: boolean;
}>();

const { t, locale } = useI18n();

const isCzk = computed(() => model.value.currency === "CZK");
const isProforma = computed(() => model.value.docType === "proforma");
/** A native correction keeps its original's currency, VAT mode and rate; an imported one is entered as printed. */
const lockedCorrection = computed(() => isCorrection(model.value.docType) && !model.value.imported);
const accounts = computed(() => props.bankAccounts.filter((a) => a.currency === model.value.currency));
const accountLabel = (a: BankAccount) => [a.label, a.accountNumber ?? a.iban].filter(Boolean).join(" — ");

function onCurrency(event: Event) {
  model.value = changeCurrency(model.value, (event.target as HTMLInputElement).value, props.bankAccounts);
}
const selected = (event: Event) => (event.target as HTMLSelectElement).value;
function onPaymentMethod(event: Event) {
  model.value = changePaymentMethod(model.value, selected(event) as PaymentMethod, props.company);
}
function onRounding(event: Event) {
  model.value = changeRounding(model.value, selected(event) as RoundingMode);
}
function onLocale(event: Event) {
  model.value = changeLocale(model.value, selected(event) as DocLocale, props.company);
}

const cls = (field: string) => ({ "input-error": props.errors[field] });
</script>

<template>
  <div class="grid gap-4 sm:grid-cols-3">
    <FormField :label="t('documents.fields.issueDate')" for="doc-issueDate" :error="errors.issueDate">
      <input id="doc-issueDate" v-model="model.issueDate" type="date" class="input" :class="cls('issueDate')" />
    </FormField>
    <FormField v-if="!isProforma" :label="t('documents.fields.taxPointDate')" for="doc-taxPointDate" :error="errors.taxPointDate">
      <input id="doc-taxPointDate" v-model="model.taxPointDate" type="date" class="input" :class="cls('taxPointDate')" />
    </FormField>
    <FormField :label="t('documents.fields.dueDate')" for="doc-dueDate" :error="errors.dueDate">
      <input id="doc-dueDate" v-model="model.dueDate" type="date" class="input" :class="cls('dueDate')" />
    </FormField>

    <FormField :label="t('documents.fields.currency')" for="doc-currency" :error="errors.currency">
      <input id="doc-currency" :value="model.currency" maxlength="3" class="input uppercase" :class="cls('currency')" autocomplete="off" :disabled="lockedCorrection" @change="onCurrency" />
    </FormField>
    <FormField :label="t('documents.fields.roundingMode')" for="doc-roundingMode" :error="errors.roundingMode">
      <select id="doc-roundingMode" :value="model.roundingMode" class="input" :class="cls('roundingMode')" @change="onRounding">
        <option v-for="m in ROUNDING_MODES" :key="m" :value="m">{{ t(`documents.roundingMode.${m}`) }}</option>
      </select>
    </FormField>
    <div v-if="!isCzk && lockedCorrection" class="space-y-1" data-test="original-rate">
      <span class="text-sm font-medium">{{ t("documents.fields.exchangeRate") }}</span>
      <p class="py-2 text-sm">
        {{ model.exchangeRate ? formatNumber(model.exchangeRate, locale, 6) : "—" }}
        <span class="text-xs text-gray-600 dark:text-gray-400">({{ t("documents.editor.originalRate") }})</span>
      </p>
    </div>
    <div v-else-if="!isCzk" class="space-y-1">
      <FormField :label="t('documents.editor.manualRate')" for="doc-exchangeRate" :error="errors.exchangeRate" :hint="t('documents.editor.manualRateHint')">
        <input id="doc-exchangeRate" v-model="model.exchangeRate" inputmode="decimal" class="input" :class="cls('exchangeRate')" autocomplete="off" />
      </FormField>
      <IndicativeRateNote
        :rate="indicativeRate"
        :error="indicativeError"
        :loading="indicativeLoading"
        :currency="model.currency"
        message-key="documents.editor.indicativeRate"
      />
    </div>
    <FormField :label="t('documents.fields.vatMode')" for="doc-vatMode" :error="errors.vatMode">
      <select id="doc-vatMode" v-model="model.vatMode" class="input" :disabled="lockedCorrection">
        <option v-for="m in VAT_MODES" :key="m" :value="m">{{ t(`documents.vatMode.${m}`) }}</option>
      </select>
    </FormField>
    <FormField :label="t('documents.fields.locale')" for="doc-locale" :error="errors.locale">
      <select id="doc-locale" :value="model.locale" class="input" @change="onLocale">
        <option v-for="code in LOCALES" :key="code" :value="code">{{ t(`locale.names.${code}`) }}</option>
      </select>
    </FormField>
    <FormField :label="t('documents.fields.paymentMethod')" for="doc-paymentMethod" :error="errors.paymentMethod">
      <select id="doc-paymentMethod" :value="model.paymentMethod" class="input" @change="onPaymentMethod">
        <option v-for="m in PAYMENT_METHODS" :key="m" :value="m">{{ t(`documents.paymentMethod.${m}`) }}</option>
      </select>
    </FormField>

    <FormField class="sm:col-span-3" :label="t('documents.fields.bankAccountId')" for="doc-bankAccountId" :error="errors.bankAccountId">
      <select id="doc-bankAccountId" v-model="model.bankAccountId" class="input" :class="cls('bankAccountId')">
        <option value="">{{ accounts.length ? t("common.notSet") : t("documents.editor.noAccountForCurrency", { currency: model.currency }) }}</option>
        <option v-for="a in accounts" :key="a.id" :value="a.id">{{ accountLabel(a) }}</option>
      </select>
    </FormField>

    <FormField :label="t('documents.fields.variableSymbol')" for="doc-variableSymbol" :error="errors.variableSymbol" :hint="t('documents.editor.variableSymbolHint')">
      <input id="doc-variableSymbol" v-model="model.variableSymbol" inputmode="numeric" maxlength="10" class="input" :class="cls('variableSymbol')" />
    </FormField>
    <FormField :label="t('documents.fields.constantSymbol')" for="doc-constantSymbol" :error="errors.constantSymbol">
      <input id="doc-constantSymbol" v-model="model.constantSymbol" inputmode="numeric" maxlength="4" class="input" :class="cls('constantSymbol')" />
    </FormField>
    <FormField :label="t('documents.fields.orderRef')" for="doc-orderRef" :error="errors.orderRef">
      <input id="doc-orderRef" v-model="model.orderRef" maxlength="100" class="input" :class="cls('orderRef')" />
    </FormField>
  </div>
</template>
