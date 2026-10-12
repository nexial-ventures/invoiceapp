<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { remainingAmount, toCents } from "@/lib/money";
import { ApiError } from "@/api/client";
import FormField from "@/components/form/FormField.vue";
import { useAction } from "@/composables/useAction";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { collectErrors, nullIfEmpty, textRule } from "@/lib/formErrors";
import { todayIso } from "../form";
import { formatDate, formatMoney, formatNumber } from "@/lib/format";
import { detailLocation } from "../routes";
import { useDocumentStore } from "../store";
import type { Document, Payment } from "../types";
import { useIndicativeRate } from "../useIndicativeRate";
import { useSessionStore } from "@/stores/session";

const props = defineProps<{ doc: Document }>();

const { t, locale } = useI18n();
const session = useSessionStore();
const store = useDocumentStore();
const { fieldErrors, error, submitting, submit } = useFormSubmit();
const { error: removeError, run } = useAction();

const editable = computed(() => props.doc.status === "issued" && session.can("write"));
// A settled invoice rarely takes another payment; the form (defaulting to 0.00) stays behind a toggle.
const settled = computed(() => props.doc.paymentState === "paid" || props.doc.paymentState === "overpaid");
const addAnother = ref(false);
const remaining = computed(() => remainingAmount(props.doc.totals.payable, props.doc.paid));
const money = (v: string) => formatMoney(v, props.doc.currency, locale.value);
/**
 * A foreign-currency proforma payment may fix the DDPP's rate; empty = ČNB for the payment date.
 * Received and imported proformas never create a DDPP.
 */
const withRate = computed(
  () => props.doc.docType === "proforma" && props.doc.currency !== "CZK" && props.doc.direction === "issued" && !props.doc.imported,
);

const date = ref(todayIso());
const amount = ref(remaining.value);
const note = ref("");
const rate = ref("");
/** The DDPP issued by the last added payment. */
const createdDdpp = ref<string | null>(null);
/** The draft final invoice blocking a payment delete (409 `advance_in_use`). */
const blockingDraft = ref<string | null>(null);
// The default follows the outstanding amount after each payment change.
watch(remaining, (value) => (amount.value = value));

const indicative = useIndicativeRate(() => ({ currency: withRate.value ? props.doc.currency : "", date: date.value }));

/** DDPP number by id, from the proforma's related documents. */
const ddpp = (id: string | null) => (id ? props.doc.relatedDocuments.find((d) => d.id === id) : undefined);
const ddppLabel = (id: string) => ddpp(id)?.number ?? t("documents.draftNumber");

async function onSubmit() {
  const value = amount.value.trim().replace(",", ".");
  const manualRate = withRate.value ? nullIfEmpty(rate.value)?.replace(",", ".") ?? null : null;
  createdDdpp.value = null;
  await submit(
    () =>
      collectErrors({
        date: !/^\d{4}-\d{2}-\d{2}$/.test(date.value) && "required",
        amount: !/^\d+(\.\d{1,2})?$/.test(value) || (toCents(value) ?? 0n) <= 0n ? "invalid" : null,
        note: textRule(note.value, { max: 500 }),
        exchangeRate: manualRate !== null && (!/^\d+(\.\d{1,6})?$/.test(manualRate) || Number(manualRate) <= 0) && "invalid",
      }),
    async () => {
      const payment = await store.addPayment({
        date: date.value,
        amount: value,
        note: nullIfEmpty(note.value),
        ...(withRate.value ? { exchangeRate: manualRate } : {}),
      });
      createdDdpp.value = payment.advanceDocumentId;
      note.value = "";
      rate.value = "";
    },
  );
}

function remove(p: Payment) {
  const message = p.advanceDocumentId
    ? t("documents.payments.confirmDeleteWithDdpp", { number: ddppLabel(p.advanceDocumentId) })
    : t("documents.payments.confirmDelete");
  if (!window.confirm(message)) return;
  createdDdpp.value = null;
  blockingDraft.value = null;
  void run(async () => {
    try {
      await store.removePayment(p.id);
    } catch (err) {
      if (err instanceof ApiError && err.code === "advance_in_use") {
        blockingDraft.value = props.doc.relatedDocuments.find((d) => d.docType === "invoice" && d.status === "draft")?.id ?? null;
      }
      throw err;
    }
  });
}
</script>

<template>
  <section class="card space-y-4" data-test="payments-panel">
    <div class="flex flex-wrap items-baseline justify-between gap-2">
      <h2 class="font-semibold">{{ doc.sign === -1 ? t("documents.payments.refundsTitle") : t("documents.payments.title") }}</h2>
      <p class="text-sm">
        {{ t("documents.payments.paid") }}: <span class="tabular-nums" data-test="paid">{{ money(doc.paid) }}</span>
        · {{ t("documents.payments.remaining") }}: <span class="tabular-nums font-medium" data-test="remaining">{{ money(remaining) }}</span>
      </p>
    </div>

    <table v-if="store.payments.length" class="table">
      <thead>
        <tr>
          <th>{{ t("documents.fields.date") }}</th>
          <th class="text-right">{{ t("documents.fields.amount") }}</th>
          <th class="hidden sm:table-cell">{{ t("documents.fields.note") }}</th>
          <th v-if="editable" class="w-0"><span class="sr-only">{{ t("common.actions") }}</span></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="p in store.payments" :key="p.id" data-test="payment-row">
          <td class="whitespace-nowrap">{{ formatDate(p.date, locale) }}</td>
          <td class="text-right tabular-nums">{{ money(p.amount) }}</td>
          <td class="hidden sm:table-cell">
            {{ p.note }}
            <RouterLink v-if="p.advanceDocumentId" :to="detailLocation(p.advanceDocumentId)" class="block text-xs text-blue-600 hover:underline dark:text-blue-400" data-test="payment-ddpp">
              {{ t("documents.payments.ddpp", { number: ddppLabel(p.advanceDocumentId) }) }}
            </RouterLink>
          </td>
          <td v-if="editable">
            <button type="button" class="btn btn-sm btn-danger" :aria-label="t('documents.payments.delete')" data-test="delete-payment" @click="remove(p)">✕</button>
          </td>
        </tr>
      </tbody>
    </table>
    <p v-else class="text-sm text-gray-500">{{ t("documents.payments.empty") }}</p>
    <p v-if="removeError" role="alert" class="alert-error" data-test="payment-error">
      {{ removeError }}
      <RouterLink v-if="blockingDraft" :to="detailLocation(blockingDraft)" class="ml-1 font-medium underline" data-test="blocking-draft">
        {{ t("documents.payments.openDraftInvoice") }}
      </RouterLink>
    </p>
    <p v-if="createdDdpp" class="rounded-md bg-green-50 px-3 py-2 text-sm text-green-800 dark:bg-green-950/50 dark:text-green-300" data-test="ddpp-created">
      {{ t("documents.payments.ddppCreated") }}
      <RouterLink :to="detailLocation(createdDdpp)" class="font-medium underline">{{ ddppLabel(createdDdpp) }}</RouterLink>
    </p>

    <button v-if="editable && settled && !addAnother" type="button" class="btn btn-sm" data-test="add-another" @click="addAnother = true">
      {{ t("documents.payments.addAnother") }}
    </button>
    <form v-if="editable && (!settled || addAnother)" class="grid gap-3 border-t border-gray-200 pt-4 sm:grid-cols-[auto_auto_1fr_auto] sm:items-end dark:border-gray-800" novalidate @submit.prevent="onSubmit">
      <FormField :label="t('documents.fields.date')" for="payment-date" :error="fieldErrors.date">
        <input id="payment-date" v-model="date" type="date" class="input" :class="{ 'input-error': fieldErrors.date }" />
      </FormField>
      <FormField :label="t('documents.fields.amount')" for="payment-amount" :error="fieldErrors.amount">
        <input id="payment-amount" v-model="amount" inputmode="decimal" class="input" :class="{ 'input-error': fieldErrors.amount }" />
      </FormField>
      <FormField :label="t('documents.fields.note')" for="payment-note" :error="fieldErrors.note">
        <input id="payment-note" v-model="note" maxlength="500" class="input" />
      </FormField>
      <button type="submit" class="btn btn-primary" :disabled="submitting" data-test="add-payment">{{ t("documents.payments.add") }}</button>
      <div v-if="withRate" class="space-y-1 sm:col-span-4">
        <FormField :label="t('documents.payments.rate')" for="payment-rate" :error="fieldErrors.exchangeRate" :hint="t('documents.payments.rateHint')">
          <input id="payment-rate" v-model="rate" inputmode="decimal" class="input sm:max-w-xs" :class="{ 'input-error': fieldErrors.exchangeRate }" autocomplete="off" />
        </FormField>
        <p v-if="indicative.rate.value" class="text-xs text-gray-600 dark:text-gray-400" data-test="payment-indicative-rate">
          {{ t("documents.payments.indicativeRate", { date: formatDate(indicative.rate.value.date, locale), rate: formatNumber(indicative.rate.value.rate, locale, 6) }) }}
        </p>
      </div>
      <p v-if="error" role="alert" class="alert-error sm:col-span-4">{{ error }}</p>
    </form>
  </section>
</template>
