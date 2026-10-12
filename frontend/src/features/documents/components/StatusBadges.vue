<script setup lang="ts">
import { useI18n } from "vue-i18n";
import type { Direction } from "@/api/types";
import type { DocStatus, PaymentState } from "../types";

defineProps<{
  status: DocStatus;
  paymentState: PaymentState | null;
  overdue: boolean;
  sentAt?: string | null;
  /** -1 marks a credit note / DDPP correction (`docType` picks the badge label, default credit note). */
  sign?: 1 | -1;
  docType?: string;
  /** A received document's `issued` reads "Recorded". */
  direction?: Direction;
  imported?: boolean;
}>();

const { t } = useI18n();

const STATUS_CLASS: Record<DocStatus, string> = {
  draft: "!bg-gray-200 !text-gray-800 dark:!bg-gray-700 dark:!text-gray-200",
  issued: "",
  cancelled: "!bg-gray-100 !text-gray-500 line-through dark:!bg-gray-800 dark:!text-gray-400",
};
const PAYMENT_CLASS: Record<PaymentState, string> = {
  unpaid: "!bg-amber-100 !text-amber-800 dark:!bg-amber-900/50 dark:!text-amber-300",
  partial: "!bg-yellow-100 !text-yellow-800 dark:!bg-yellow-900/50 dark:!text-yellow-300",
  paid: "!bg-green-100 !text-green-800 dark:!bg-green-900/50 dark:!text-green-300",
  overpaid: "!bg-purple-100 !text-purple-800 dark:!bg-purple-900/50 dark:!text-purple-300",
};
</script>

<template>
  <span class="inline-flex flex-wrap gap-1">
    <span v-if="sign === -1" class="badge !bg-rose-100 !text-rose-800 dark:!bg-rose-900/50 dark:!text-rose-300" data-test="credit-note-badge">
      {{ t(`documents.docTypes.${docType === "advance_credit_note" ? docType : "credit_note"}`) }}
    </span>
    <span class="badge" :class="STATUS_CLASS[status]" data-test="status-badge">
      {{ direction === "received" && status === "issued" ? t("documents.status.recorded") : t(`documents.status.${status}`) }}
    </span>
    <span v-if="imported" class="badge !bg-indigo-100 !text-indigo-800 dark:!bg-indigo-900/50 dark:!text-indigo-300" data-test="imported-badge">
      {{ t("documents.imported") }}
    </span>
    <span v-if="paymentState" class="badge" :class="PAYMENT_CLASS[paymentState]" data-test="payment-badge">
      {{ t(`documents.paymentState.${paymentState}`) }}
    </span>
    <span v-if="overdue" class="badge !bg-red-100 !text-red-800 dark:!bg-red-900/50 dark:!text-red-300" data-test="overdue-badge">
      {{ t("documents.overdue") }}
    </span>
    <span v-if="sentAt && status === 'issued'" class="badge !bg-sky-100 !text-sky-800 dark:!bg-sky-900/50 dark:!text-sky-300">
      {{ t("documents.sent") }}
    </span>
  </span>
</template>
