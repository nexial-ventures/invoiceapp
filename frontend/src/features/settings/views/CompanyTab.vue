<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import PartyFields from "@/components/form/PartyFields.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { LOCALES } from "@/i18n";
import DocumentDefaultsSection from "../components/DocumentDefaultsSection.vue";
import { toCompany, toCompanyDraft, validateCompany } from "../companyForm";
import { useCompanyStore } from "../stores";

const { t } = useI18n();
const store = useCompanyStore();
const draft = ref(toCompanyDraft(store.company));
const saved = ref(false);
// The form stays disabled until the reload lands, so replacing the draft with
// fresh server data can never discard edits typed in the meantime.
const loaded = ref(false);
const { fieldErrors, error, submitting, submit, showError } = useFormSubmit();

onMounted(async () => {
  try {
    await store.load();
    draft.value = toCompanyDraft(store.company);
  } catch (err) {
    showError(err);
  } finally {
    // On failure fall back to the cached copy if there is one.
    loaded.value = store.company !== null;
  }
});

async function onSubmit() {
  saved.value = false;
  saved.value = await submit(
    () => validateCompany(draft.value),
    async () => {
      await store.save(toCompany(draft.value));
      draft.value = toCompanyDraft(store.company);
    },
  );
}
</script>

<template>
  <form novalidate @submit.prevent="onSubmit">
    <fieldset class="space-y-6" :disabled="!loaded">
    <section class="card space-y-4">
      <PartyFields v-model="draft" :errors="fieldErrors" id-prefix="company" />
      <label class="flex items-center gap-2 text-sm">
        <input v-model="draft.vatPayer" type="checkbox" class="size-4 rounded" />
        {{ t("settings.company.vatPayer") }}
      </label>
    </section>

    <section class="card grid gap-4 sm:grid-cols-2">
      <FormField :label="t('party.email')" for="company-email" :error="fieldErrors.email">
        <input id="company-email" v-model="draft.email" type="email" class="input" />
      </FormField>
      <FormField :label="t('party.phone')" for="company-phone" :error="fieldErrors.phone">
        <input id="company-phone" v-model="draft.phone" type="tel" class="input" />
      </FormField>
      <FormField class="sm:col-span-2" :label="t('settings.company.web')" for="company-web" :error="fieldErrors.web">
        <input id="company-web" v-model="draft.web" type="url" class="input" />
      </FormField>
      <FormField
        class="sm:col-span-2"
        :label="t('settings.company.registration')"
        for="company-registration"
        :error="fieldErrors.registration"
        :hint="t('settings.company.registrationHint')"
      >
        <textarea id="company-registration" v-model="draft.registration" rows="2" maxlength="300" class="input" />
      </FormField>
      <FormField :label="t('settings.company.defaultDueDays')" for="company-due" :error="fieldErrors.defaultDueDays">
        <input id="company-due" v-model="draft.defaultDueDays" inputmode="numeric" class="input" :class="{ 'input-error': fieldErrors.defaultDueDays }" />
      </FormField>
      <FormField :label="t('settings.company.defaultLocale')" for="company-locale" :error="fieldErrors.defaultLocale">
        <select id="company-locale" v-model="draft.defaultLocale" class="input">
          <option v-for="code in LOCALES" :key="code" :value="code">{{ t(`locale.names.${code}`) }}</option>
        </select>
      </FormField>
    </section>

    <DocumentDefaultsSection v-model="draft" :errors="fieldErrors" />

    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    <div class="flex items-center gap-3">
      <button type="submit" class="btn btn-primary" :disabled="submitting">
        {{ submitting ? t("common.saving") : t("common.save") }}
      </button>
      <span v-if="saved" class="text-sm text-green-700 dark:text-green-400" role="status">{{ t("common.saved") }}</span>
    </div>
    </fieldset>
  </form>
</template>
