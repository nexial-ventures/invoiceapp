<script setup lang="ts">
import { reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { authApi } from "../api";
import AuthCard from "../components/AuthCard.vue";
import GoogleButton from "../components/GoogleButton.vue";
import { validateRegistration } from "../validation";

const { t, locale } = useI18n();
const form = reactive({ email: "", displayName: "", password: "" });
const done = ref(false);
const { fieldErrors, error, submitting, submit } = useFormSubmit();

async function onSubmit() {
  done.value = await submit(
    () => validateRegistration(form),
    () => authApi.register({ email: form.email.trim(), displayName: form.displayName.trim(), password: form.password, locale: locale.value }),
  );
}
</script>

<template>
  <AuthCard :title="t('auth.register.title')">
    <!-- 202 for a new and an existing e-mail alike (no account enumeration): one message for both. -->
    <p v-if="done" role="status" class="text-sm" data-test="register-done">{{ t("auth.register.done") }}</p>
    <form v-else class="space-y-4" novalidate @submit.prevent="onSubmit">
      <FormField :label="t('auth.email')" for="register-email" :error="fieldErrors.email">
        <input id="register-email" v-model="form.email" type="email" autocomplete="email" class="input" :class="{ 'input-error': fieldErrors.email }" data-test="register-email" />
      </FormField>
      <FormField :label="t('auth.register.displayName')" for="register-name" :error="fieldErrors.displayName">
        <input id="register-name" v-model="form.displayName" autocomplete="name" maxlength="100" class="input" :class="{ 'input-error': fieldErrors.displayName }" data-test="register-name" />
      </FormField>
      <FormField :label="t('auth.password')" for="register-password" :error="fieldErrors.password" :hint="t('auth.passwordHint')">
        <input id="register-password" v-model="form.password" type="password" autocomplete="new-password" class="input" :class="{ 'input-error': fieldErrors.password }" data-test="register-password" />
      </FormField>
      <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
      <button type="submit" class="btn btn-primary w-full" :disabled="submitting" data-test="register-submit">
        {{ submitting ? t("auth.register.submitting") : t("auth.register.submit") }}
      </button>
    </form>
    <!-- The route is reachable only with registration on; Google sign-up is a "login" that creates the account. -->
    <GoogleButton v-if="!done" purpose="login" :label="t('google.button.register')" return-to="/" divider />
    <RouterLink :to="{ name: 'login' }" class="block text-sm text-blue-600 hover:underline dark:text-blue-400">
      {{ done ? t("auth.toLogin") : t("auth.register.haveAccount") }}
    </RouterLink>
  </AuthCard>
</template>
