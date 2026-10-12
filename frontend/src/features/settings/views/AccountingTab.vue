<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { toAccountingDraft, toAccountingSettings, validateAccounting } from "../accounting";
import AccountingProgramSection from "../components/AccountingProgramSection.vue";
import { useAccountingStore, useCompanyStore } from "../stores";
import { ACCOUNTING_PROGRAMS } from "../types";

const { t } = useI18n();
const store = useAccountingStore();
const companyStore = useCompanyStore();
const draft = ref(toAccountingDraft(store.settings));
const saved = ref(false);
// Disabled until the reload lands, so fresh server data never replaces edits typed meanwhile.
const loaded = ref(false);
const { fieldErrors, error, submitting, submit, showError } = useFormSubmit();

onMounted(async () => {
  // The company IČO is only the override's placeholder: its failure must not block the tab.
  companyStore.ensure().catch(() => {});
  try {
    await store.load();
    draft.value = toAccountingDraft(store.settings);
  } catch (err) {
    showError(err);
  } finally {
    loaded.value = store.settings !== null;
  }
});

async function onSubmit() {
  saved.value = false;
  saved.value = await submit(
    // Without loaded limits the form is disabled; the server checks the lengths anyway.
    () => (store.settings ? validateAccounting(draft.value, store.settings.limits) : {}),
    async () => {
      await store.save(toAccountingSettings(draft.value));
      draft.value = toAccountingDraft(store.settings);
    },
  );
}
</script>

<template>
  <form novalidate @submit.prevent="onSubmit">
    <fieldset class="space-y-6" :disabled="!loaded">
      <AccountingProgramSection
        v-for="program in ACCOUNTING_PROGRAMS"
        :key="program"
        v-model="draft[program]"
        :program="program"
        :errors="fieldErrors"
        :ico-placeholder="companyStore.company?.ico ?? ''"
      />

      <p v-if="error" role="alert" class="alert-error" data-test="accounting-error">{{ error }}</p>
      <div class="flex items-center gap-3">
        <button type="submit" class="btn btn-primary" :disabled="submitting" data-test="accounting-save">
          {{ submitting ? t("common.saving") : t("common.save") }}
        </button>
        <span v-if="saved" class="text-sm text-green-700 dark:text-green-400" role="status">{{ t("common.saved") }}</span>
      </div>
    </fieldset>
  </form>
</template>
