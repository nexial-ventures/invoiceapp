<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import { formatMoney } from "@/lib/format";
import { useDocumentStore } from "../store";
import type { Document } from "../types";

/** "Unlock" on an issued document (admin+): back to a draft after a confirmation with a required reason. */
const props = defineProps<{ doc: Document }>();

const { t, locale } = useI18n();
const store = useDocumentStore();
const { error, run } = useAction();

const open = ref(false);
const reason = ref("");
const busy = ref(false);
const missing = ref(false);

const paid = computed(() => Number(props.doc.paid) > 0);
const referenced = computed(() => props.doc.relatedDocuments.length > 0);

function start() {
  open.value = true;
  reason.value = "";
  missing.value = false;
  error.value = null;
}

async function confirm() {
  const text = reason.value.trim();
  missing.value = !text;
  if (missing.value || busy.value) return;
  busy.value = true;
  const ok = await run(() => store.unlock(text));
  busy.value = false;
  if (ok) open.value = false;
}
</script>

<template>
  <div class="space-y-2">
    <button v-if="!open" type="button" class="btn" data-test="unlock" @click="start">{{ t("unlock.action") }}</button>
    <section v-else role="dialog" aria-labelledby="unlock-title" class="card space-y-3" data-test="unlock-dialog">
      <h2 id="unlock-title" class="text-lg font-semibold">{{ t("unlock.title", { number: doc.number ?? "" }) }}</h2>
      <p class="text-sm">{{ t("unlock.intro") }}</p>
      <ul v-if="doc.sentAt || paid || referenced" class="list-disc space-y-1 rounded-md bg-amber-50 py-2 pr-3 pl-8 text-sm text-amber-800 dark:bg-amber-950/50 dark:text-amber-300">
        <li v-if="doc.sentAt" data-test="unlock-warn-sent">{{ t("unlock.warnSent") }}</li>
        <li v-if="paid" data-test="unlock-warn-payments">{{ t("unlock.warnPayments", { paid: formatMoney(doc.paid, doc.currency, locale) }) }}</li>
        <li v-if="referenced" data-test="unlock-warn-related">{{ t("unlock.warnRelated") }}</li>
      </ul>
      <label for="unlock-reason" class="block text-sm font-medium">{{ t("unlock.reason") }}</label>
      <textarea
        id="unlock-reason"
        v-model="reason"
        rows="3"
        maxlength="2000"
        required
        class="input"
        :class="{ 'input-error': missing }"
        :placeholder="t('unlock.reasonPlaceholder')"
        data-test="unlock-reason"
      />
      <p v-if="missing" class="text-sm text-red-600 dark:text-red-400" data-test="unlock-reason-error">{{ t("unlock.reasonRequired") }}</p>
      <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
      <div class="flex gap-2">
        <button type="button" class="btn btn-danger" :disabled="busy" data-test="unlock-confirm" @click="confirm">{{ t("unlock.confirm") }}</button>
        <button type="button" class="btn" :disabled="busy" @click="open = false">{{ t("common.cancel") }}</button>
      </div>
    </section>
  </div>
</template>
