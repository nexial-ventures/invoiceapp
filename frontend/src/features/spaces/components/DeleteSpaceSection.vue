<script setup lang="ts">
import { computed, reactive } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import CodeField from "@/features/auth/components/CodeField.vue";
import NoPasswordHint from "@/features/auth/components/NoPasswordHint.vue";
import { codeRule, passwordFieldReason } from "@/features/auth/validation";
import AccountantExportButton from "@/features/csvExport/components/AccountantExportButton.vue";
import { collectErrors } from "@/lib/formErrors";
import { leaveTo } from "@/lib/navigate";
import { useSessionStore } from "@/stores/session";
import { spacesApi } from "../api";

const props = defineProps<{ slug: string }>();
const { t } = useI18n();
const session = useSessionStore();
const form = reactive({ slug: "", password: "", code: "" });
const { fieldErrors, error, submitting, submit } = useFormSubmit();
const confirmed = computed(() => form.slug.trim() === props.slug && form.password !== "" && (!session.mfaEnabled || form.code.trim() !== ""));

async function onSubmit() {
  const mfa = session.mfaEnabled;
  const ok = await submit(
    () =>
      collectErrors({
        slug: form.slug.trim() !== props.slug && "mismatch",
        password: form.password === "" && "required",
        code: mfa && codeRule(form.code),
      }),
    () => spacesApi.remove({ slug: form.slug.trim(), password: form.password, ...(mfa ? { code: form.code.trim() } : {}) }),
  );
  // The space, its sessions and this host's data are gone: back to the hub.
  if (ok) leaveTo(session.baseUrl);
}
</script>

<template>
  <section class="card space-y-4 border-red-300 dark:border-red-900" data-test="delete-space">
    <h2 class="text-lg font-semibold text-red-700 dark:text-red-400">{{ t("spaces.delete.title") }}</h2>
    <p class="alert-error" data-test="delete-warning">{{ t("spaces.delete.warning") }}</p>
    <div class="flex flex-wrap items-center gap-2 text-sm">
      <span>{{ t("spaces.delete.exportHint") }}</span>
      <AccountantExportButton />
    </div>
    <NoPasswordHint v-if="!session.hasPassword" />
    <form v-else class="space-y-4" novalidate @submit.prevent="onSubmit">
      <FormField :label="t('spaces.delete.confirmSlug', { slug })" for="delete-slug" :error="fieldErrors.slug">
        <input id="delete-slug" v-model="form.slug" autocomplete="off" spellcheck="false" class="input font-mono" :class="{ 'input-error': fieldErrors.slug }" data-test="delete-slug" />
      </FormField>
      <FormField :label="t('spaces.delete.password')" for="delete-password" :error="passwordFieldReason(fieldErrors.password)">
        <input id="delete-password" v-model="form.password" type="password" autocomplete="current-password" class="input" :class="{ 'input-error': fieldErrors.password }" data-test="delete-password" />
      </FormField>
      <CodeField v-if="session.mfaEnabled" id="delete-code" v-model="form.code" :error="fieldErrors.code" />
      <p v-if="error" role="alert" class="alert-error" data-test="delete-error">{{ error }}</p>
      <button type="submit" class="btn btn-danger" :disabled="!confirmed || submitting" data-test="delete-submit">
        {{ submitting ? t("spaces.delete.submitting") : t("spaces.delete.submit") }}
      </button>
    </form>
  </section>
</template>
