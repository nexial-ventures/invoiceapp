<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { collectErrors } from "@/lib/formErrors";
import { useSessionStore } from "@/stores/session";
import { useToastStore } from "@/stores/toast";
import { authApi } from "../api";
import { codeRule, passwordFieldReason, passwordRule } from "../validation";
import CodeField from "./CodeField.vue";
import NoPasswordHint from "./NoPasswordHint.vue";

/**
 * Account → password: "Změna hesla", or "Nastavit heslo" (no current password) for a Google-only
 * account; the server accepts the latter only shortly after a Google sign-in.
 */
const { t } = useI18n();
const session = useSessionStore();
const toast = useToastStore();
const form = reactive({ currentPassword: "", newPassword: "", code: "" });
const { fieldErrors, error, submitting, submit } = useFormSubmit();
const setting = computed(() => !session.hasPassword);
/** 422 `currentPassword: required` on setting: the 10-minute window after the Google sign-in is over. */
const windowOver = ref(false);

async function onSubmit() {
  const mfa = session.mfaEnabled;
  const first = setting.value;
  const ok = await submit(
    () =>
      collectErrors({
        currentPassword: !first && form.currentPassword === "" && "required",
        newPassword: passwordRule(form.newPassword),
        code: mfa && codeRule(form.code),
      }),
    () =>
      authApi.changePassword({
        ...(first ? {} : { currentPassword: form.currentPassword }),
        newPassword: form.newPassword,
        ...(mfa ? { code: form.code.trim() } : {}),
      }),
  );
  windowOver.value = first && fieldErrors.value.currentPassword === "required";
  if (!ok) return;
  form.currentPassword = "";
  form.newPassword = "";
  form.code = "";
  if (first) session.patchUser({ hasPassword: true });
  toast.show(first ? t("google.setPassword.done") : t("account.password.changed"));
}
</script>

<template>
  <form class="card space-y-4" novalidate :data-test="setting ? 'set-password-form' : 'password-form'" @submit.prevent="onSubmit">
    <h2 class="text-lg font-semibold">{{ setting ? t("google.setPassword.title") : t("account.password.title") }}</h2>
    <p v-if="setting" class="text-sm text-gray-600 dark:text-gray-400">{{ t("google.setPassword.intro") }}</p>
    <!-- Hidden username lets password managers pair the new password with the account. -->
    <input type="email" autocomplete="username" :value="session.me?.user.email" class="hidden" readonly />
    <FormField v-if="!setting" :label="t('account.password.current')" for="account-current" :error="passwordFieldReason(fieldErrors.currentPassword)">
      <input id="account-current" v-model="form.currentPassword" type="password" autocomplete="current-password" class="input" :class="{ 'input-error': fieldErrors.currentPassword }" data-test="current-password" />
    </FormField>
    <FormField :label="t('account.password.new')" for="account-new" :error="fieldErrors.newPassword" :hint="t('auth.passwordHint')">
      <input id="account-new" v-model="form.newPassword" type="password" autocomplete="new-password" class="input" :class="{ 'input-error': fieldErrors.newPassword }" data-test="new-password" />
    </FormField>
    <CodeField v-if="session.mfaEnabled" id="account-code" v-model="form.code" :error="fieldErrors.code" />
    <template v-if="windowOver">
      <p role="alert" class="alert-error" data-test="set-password-expired">{{ t("google.setPassword.expired") }}</p>
      <NoPasswordHint />
    </template>
    <p v-else-if="error" role="alert" class="alert-error" data-test="password-error">{{ error }}</p>
    <button type="submit" class="btn btn-primary" :disabled="submitting" data-test="change-password">
      {{ setting ? t("google.setPassword.submit") : t("account.password.submit") }}
    </button>
  </form>
</template>
