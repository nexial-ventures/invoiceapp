<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { ApiError } from "@/api/client";
import ErrorDetail from "@/components/ErrorDetail.vue";
import { useErrorText } from "@/composables/useAction";
import { errorDetailOf, fieldErrorsOf } from "@/lib/formErrors";
import { canCorrectDdpp, canCreditOrDebit } from "../docTypes";
import { describeFieldErrors } from "../fieldMessages";
import { editLocation, listLocation } from "../routes";
import { useDocumentStore } from "../store";
import type { Document } from "../types";

const props = defineProps<{ doc: Document }>();

const { t } = useI18n();
const router = useRouter();
const store = useDocumentStore();
const errorText = useErrorText();

const busy = ref(false);
const error = ref<string | null>(null);
const reasons = ref<string[]>([]);
const detail = ref<string | null>(null);

// Issue renders + archives the PDF in its transaction: a PDF failure means nothing was issued.
const ISSUE_PDF_MESSAGES: Record<string, string> = {
  pdf_unavailable: "pdf.issueUnavailable",
  pdf_render_failed: "pdf.issueRenderFailed",
  storage_unavailable: "pdf.issueStorageUnavailable",
};
// A DDPP correction is refused (create, issue, cancel) once the DDPP is deducted by an invoice.
const DDPP_CORRECTION_MESSAGES: Record<string, string> = {
  advance_settled: "documents.detail.ddppCorrectionSettled",
  advance_in_use: "documents.detail.ddppCorrectionInUse",
};
// Cancelling an (imported) DDPP: deducted by an issued invoice, or held by a draft invoice or a live correction.
const DDPP_CANCEL_MESSAGES: Record<string, string> = {
  advance_settled: "documents.detail.ddppCancelSettled",
  advance_in_use: "documents.detail.ddppCancelInUse",
};
// Cancelling a DDPP correction: its DDPP is deducted by an issued / draft final invoice.
const CORRECTION_CANCEL_MESSAGES: Record<string, string> = {
  advance_settled: "documents.detail.ddppCorrectionCancelSettled",
  advance_in_use: "documents.detail.ddppCorrectionCancelInUse",
};
const CANCEL_MESSAGES: Partial<Record<string, Record<string, string>>> = {
  advance_tax_doc: DDPP_CANCEL_MESSAGES,
  advance_credit_note: CORRECTION_CANCEL_MESSAGES,
};
const CORRECTION_MESSAGES: Record<string, string> = { invalid_state: "documents.detail.correctionInvalidState" };
const ddppRelated = computed(() => props.doc.docType === "advance_tax_doc" || props.doc.docType === "advance_credit_note");

/** Message for the codes the action explains better than the generic error text. */
const messages = (...maps: Record<string, string>[]) => (err: unknown): string | null => {
  const code = err instanceof ApiError ? err.code : "";
  const key = maps.map((m) => m[code]).find(Boolean);
  return key ? t(key) : null;
};
const ddppMessages = () => (ddppRelated.value ? DDPP_CORRECTION_MESSAGES : {});

const issued = computed(() => props.doc.status === "issued");
// A DDPP is created and cancelled by its proforma's payments; only its delivery is tracked here.
const isDdpp = computed(() => props.doc.docType === "advance_tax_doc");
// Also for imported originals: the native flows create our own drafts from them.
const canCorrect = computed(() => canCreditOrDebit(props.doc));
const canCorrectAdvance = computed(() => canCorrectDdpp(props.doc));
/** Why the server would refuse a DDPP correction right now (the button stays visible, disabled). */
const correctionBlockText = computed(() => (props.doc.correctionBlock ? t(`documents.detail.correctionBlock.${props.doc.correctionBlock}`) : null));
const canSettle = computed(() => issued.value && props.doc.docType === "proforma" && !props.doc.settled);

/** `message` overrides the generic error text for codes the action explains better. */
async function act(action: () => Promise<void>, message?: (err: unknown) => string | null): Promise<void> {
  if (busy.value) return;
  busy.value = true;
  error.value = null;
  reasons.value = [];
  detail.value = null;
  try {
    await action();
  } catch (err) {
    const fields = fieldErrorsOf(err);
    error.value = fields ? t("documents.detail.cannotProceed") : (message?.(err) ?? errorText(err));
    reasons.value = fields ? describeFieldErrors(fields, t) : [];
    detail.value = errorDetailOf(err);
  } finally {
    busy.value = false;
  }
}

// Re-issue / delete of an unlocked draft: the documents relying on it must still hold.
const UNLOCKED_ISSUE_MESSAGES: Record<string, string> = {
  exceeds_original: "unlock.issueExceeds",
  related_in_use: "unlock.issueRelated",
  advance_settled: "unlock.issueAdvance",
  advance_in_use: "unlock.issueAdvance",
};
const UNLOCKED_DELETE_MESSAGES: Record<string, string> = {
  exceeds_original: "unlock.deleteExceeds",
  related_in_use: "unlock.deleteRelated",
  advance_settled: "unlock.deleteAdvance",
  advance_in_use: "unlock.deleteAdvance",
};
const unlockedMessages = (map: Record<string, string>) => (props.doc.unlockedAt ? map : {});

function issue() {
  if (!window.confirm(t(props.doc.imported ? "documents.import.confirmIssue" : "documents.detail.confirmIssue"))) return;
  void act(store.issue, messages(unlockedMessages(UNLOCKED_ISSUE_MESSAGES), ISSUE_PDF_MESSAGES, ddppMessages()));
}

function remove() {
  // An unlocked draft was issued: its payments and e-mail log go with it.
  const question = props.doc.unlockedAt ? t("unlock.confirmDelete", { number: props.doc.number ?? "" }) : t("documents.detail.confirmDelete");
  if (!window.confirm(question)) return;
  const docType = props.doc.docType;
  void act(async () => {
    await store.remove();
    await router.push(listLocation(docType));
  }, messages(unlockedMessages(UNLOCKED_DELETE_MESSAGES)));
}

function cancel() {
  const reason = window.prompt(t("documents.detail.cancelPrompt", { number: props.doc.number ?? "" }));
  if (reason === null) return;
  void act(() => store.cancel(reason.trim() || null), messages(CANCEL_MESSAGES[props.doc.docType] ?? {}));
}

/**
 * Credit note, debit note or (on a DDPP) its correction. The reason asked here is required again
 * in the editor before issue; the new draft opens there.
 */
function correct(kind: "creditNote" | "debitNote" | "ddppCorrection") {
  const reason = window.prompt(t(`documents.detail.${kind}Prompt`, { number: props.doc.number ?? "" }));
  if (reason === null) return;
  const create = kind === "debitNote" ? store.debitNote : store.creditNote;
  void act(async () => {
    const draft = await create(reason.trim() || null);
    await router.push(editLocation(draft.id));
  }, messages(CORRECTION_MESSAGES, ddppMessages()));
}

function settle() {
  void act(async () => {
    const draft = await store.settle();
    await router.push(editLocation(draft.id));
  });
}
</script>

<template>
  <div class="space-y-2">
    <div class="flex flex-wrap gap-2">
      <template v-if="doc.status === 'draft'">
        <RouterLink :to="editLocation(doc.id)" class="btn" data-test="edit">{{ t("common.edit") }}</RouterLink>
        <button type="button" class="btn btn-primary" :disabled="busy" data-test="issue" @click="issue">{{ t("documents.detail.issue") }}</button>
        <button type="button" class="btn btn-danger" :disabled="busy" data-test="delete" @click="remove">{{ t("common.delete") }}</button>
      </template>
      <template v-else-if="issued">
        <button v-if="canSettle" type="button" class="btn btn-primary" :disabled="busy" data-test="settle" @click="settle">
          {{ t("documents.detail.settle") }}
        </button>
        <button type="button" class="btn" :disabled="busy" data-test="mark-sent" @click="act(store.markSent)">
          {{ doc.sentAt ? t("documents.detail.markSentAgain") : t("documents.detail.markSent") }}
        </button>
        <template v-if="canCorrect">
          <button type="button" class="btn" :disabled="busy" data-test="credit-note" @click="correct('creditNote')">
            {{ t("documents.detail.creditNote") }}
          </button>
          <button type="button" class="btn" :disabled="busy" data-test="debit-note" @click="correct('debitNote')">
            {{ t("documents.detail.debitNote") }}
          </button>
        </template>
        <span v-if="canCorrectAdvance" class="inline-flex flex-wrap items-center gap-2">
          <button
            type="button"
            class="btn"
            :disabled="busy || !!correctionBlockText"
            :title="correctionBlockText ?? undefined"
            :aria-describedby="correctionBlockText ? 'ddpp-correction-block' : undefined"
            data-test="ddpp-correction"
            @click="correct('ddppCorrection')"
          >
            {{ t("documents.detail.ddppCorrection") }}
          </button>
          <span v-if="correctionBlockText" id="ddpp-correction-block" class="text-xs text-gray-600 dark:text-gray-400" data-test="ddpp-correction-block">
            {{ correctionBlockText }}
          </span>
        </span>
        <button v-if="!isDdpp || doc.imported" type="button" class="btn btn-danger" :disabled="busy" data-test="cancel" @click="cancel">{{ t("documents.detail.cancel") }}</button>
      </template>
    </div>
    <div v-if="error" role="alert" class="alert-error" data-test="action-error">
      <p>{{ error }}</p>
      <ul v-if="reasons.length" class="mt-1 list-disc pl-5">
        <li v-for="r in reasons" :key="r">{{ r }}</li>
      </ul>
      <ErrorDetail :detail="detail" />
    </div>
  </div>
</template>
