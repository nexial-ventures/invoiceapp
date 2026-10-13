<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import PartyFields from "@/components/form/PartyFields.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { LOCALES } from "@/i18n";
import { toDraft, toInput, validateContact } from "../form";
import { useContactsStore } from "../store";
import type { Contact } from "../types";

/** `readonly`: the role may not change contacts (accountant); every control is disabled, no Save. */
const props = defineProps<{ contact?: Contact; readonly?: boolean }>();
const emit = defineEmits<{ saved: [contact: Contact] }>();

const { t } = useI18n();
const store = useContactsStore();
const draft = ref(toDraft(props.contact));
const { fieldErrors, error, submitting, submit } = useFormSubmit();

async function onSubmit() {
  await submit(
    () => validateContact(draft.value),
    async () => {
      emit("saved", await store.save(toInput(draft.value), props.contact?.id));
    },
  );
}
</script>

<template>
  <form class="space-y-6" novalidate @submit.prevent="readonly || onSubmit()">
    <fieldset class="space-y-6" :disabled="readonly" data-test="contact-fields">
      <section class="card space-y-4">
        <PartyFields v-model="draft" :errors="fieldErrors" id-prefix="contact" />
      </section>

      <section class="card grid gap-4 sm:grid-cols-2">
        <FormField :label="t('party.email')" for="contact-email" :error="fieldErrors.email">
          <input id="contact-email" v-model="draft.email" type="email" class="input" :class="{ 'input-error': fieldErrors.email }" />
        </FormField>
        <FormField :label="t('party.phone')" for="contact-phone" :error="fieldErrors.phone">
          <input id="contact-phone" v-model="draft.phone" type="tel" class="input" :class="{ 'input-error': fieldErrors.phone }" />
        </FormField>
        <FormField :label="t('contacts.defaultDueDays')" for="contact-due" :error="fieldErrors.defaultDueDays">
          <input id="contact-due" v-model="draft.defaultDueDays" inputmode="numeric" class="input" :class="{ 'input-error': fieldErrors.defaultDueDays }" />
        </FormField>
        <FormField :label="t('contacts.defaultCurrency')" for="contact-currency" :error="fieldErrors.defaultCurrency">
          <input id="contact-currency" v-model="draft.defaultCurrency" maxlength="3" class="input uppercase" :class="{ 'input-error': fieldErrors.defaultCurrency }" />
        </FormField>
        <FormField :label="t('contacts.defaultLocale')" for="contact-locale" :error="fieldErrors.defaultLocale">
          <select id="contact-locale" v-model="draft.defaultLocale" class="input">
            <option value="">{{ t("common.notSet") }}</option>
            <option v-for="code in LOCALES" :key="code" :value="code">{{ t(`locale.names.${code}`) }}</option>
          </select>
        </FormField>
        <FormField class="sm:col-span-2" :label="t('contacts.note')" for="contact-note" :error="fieldErrors.note">
          <textarea id="contact-note" v-model="draft.note" rows="3" class="input" />
        </FormField>
      </section>

      <section class="card space-y-4" data-test="contact-numbering">
        <h2 class="font-semibold">{{ t("numbering.contact.title") }}</h2>
        <FormField :label="t('numbering.contact.numberCode')" for="contact-numberCode" :error="fieldErrors.numberCode" :hint="t('numbering.contact.numberCodeHint')">
          <input
            id="contact-numberCode"
            v-model="draft.numberCode"
            maxlength="10"
            class="input max-w-xs font-mono"
            :class="{ 'input-error': fieldErrors.numberCode }"
            autocomplete="off"
            spellcheck="false"
          />
        </FormField>
      </section>
    </fieldset>

    <p v-if="error" role="alert" class="alert-error" data-test="form-error">{{ error }}</p>

    <div class="flex flex-wrap items-center gap-2">
      <button v-if="!readonly" type="submit" class="btn btn-primary" :disabled="submitting" data-test="contact-save">
        {{ submitting ? t("common.saving") : t("common.save") }}
      </button>
      <slot name="actions" />
    </div>
  </form>
</template>
