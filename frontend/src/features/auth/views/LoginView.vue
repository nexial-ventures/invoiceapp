<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { collectErrors, isApiError } from "@/lib/formErrors";
import { safeReturnPath } from "@/lib/returnPath";
import { useSessionStore } from "@/stores/session";
import AuthCard from "../components/AuthCard.vue";
import GoogleButton from "../components/GoogleButton.vue";
import GoogleError from "../components/GoogleError.vue";
import LoginCodeStep from "../components/LoginCodeStep.vue";
import { emailRule } from "../validation";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const session = useSessionStore();
const form = reactive({ email: "", password: "" });
const { fieldErrors, error, submitting, submit } = useFormSubmit();
// `?mfa=1`: a Google sign-in of a user with TOTP; the server already set the pending-login cookie.
const step = ref<"password" | "code">(route.query.mfa === "1" ? "code" : "password");
/** 403 `mfa_required`: this space requires TOTP and the account has none. */
const mfaRequired = ref(false);
const accountUrl = computed(() => `${session.baseUrl.replace(/\/$/, "")}/account`);

const title = computed(() => {
  const space = session.context?.space;
  return space ? t("auth.login.spaceTitle", { name: space.name }) : t("auth.login.title");
});
const canRegister = computed(() => session.context?.kind === "base" && session.context.registration);

const returnTo = computed(() => safeReturnPath(route.query.return));
const finish = () => router.replace(returnTo.value);

async function onSubmit() {
  mfaRequired.value = false;
  // `as`: assigned inside the closure, so TS must not narrow it to "done" here.
  let next = "done" as "done" | "mfa";
  const ok = await submit(
    () => collectErrors({ email: emailRule(form.email), password: form.password === "" && "required" }),
    async () => {
      try {
        next = await session.login({ email: form.email.trim(), password: form.password });
      } catch (err) {
        mfaRequired.value = isApiError(err, 403, "mfa_required");
        throw err;
      }
    },
  );
  if (!ok) return;
  if (next === "mfa") step.value = "code";
  else await finish();
}

function onExpired() {
  step.value = "password";
  form.password = "";
  error.value = t("auth.login.mfa.expired");
}
</script>

<template>
  <AuthCard :title="title">
    <GoogleError v-if="step === 'password'" />
    <LoginCodeStep v-if="step === 'code'" @done="finish" @expired="onExpired" />
    <form v-else class="space-y-4" novalidate @submit.prevent="onSubmit">
      <FormField :label="t('auth.email')" for="login-email" :error="fieldErrors.email">
        <input id="login-email" v-model="form.email" type="email" autocomplete="username" autofocus class="input" :class="{ 'input-error': fieldErrors.email }" data-test="login-email" />
      </FormField>
      <FormField :label="t('auth.password')" for="login-password" :error="fieldErrors.password">
        <input id="login-password" v-model="form.password" type="password" autocomplete="current-password" class="input" :class="{ 'input-error': fieldErrors.password }" data-test="login-password" />
      </FormField>
      <div v-if="error" role="alert" class="alert-error space-y-1" data-test="login-error">
        <p>{{ error }}</p>
        <a v-if="mfaRequired" :href="accountUrl" class="font-medium underline" data-test="login-mfa-setup">{{ t("auth.login.mfa.setUp") }}</a>
      </div>
      <button type="submit" class="btn btn-primary w-full" :disabled="submitting" data-test="login-submit">
        {{ submitting ? t("auth.login.submitting") : t("auth.login.submit") }}
      </button>
    </form>
    <GoogleButton v-if="step === 'password'" purpose="login" :label="t('google.button.login')" :return-to="returnTo" divider />
    <div class="flex flex-col gap-1 text-sm">
      <RouterLink :to="{ name: 'forgot' }" class="text-blue-600 hover:underline dark:text-blue-400">{{ t("auth.login.forgot") }}</RouterLink>
      <RouterLink v-if="canRegister" :to="{ name: 'register' }" class="text-blue-600 hover:underline dark:text-blue-400" data-test="register-link">
        {{ t("auth.login.register") }}
      </RouterLink>
    </div>
  </AuthCard>
</template>
