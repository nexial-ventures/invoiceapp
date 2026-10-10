<script setup lang="ts">
import { reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { collectErrors } from "@/lib/formErrors";
import { leaveTo } from "@/lib/navigate";
import { useSessionStore } from "@/stores/session";
import { useToastStore } from "@/stores/toast";
import { authApi } from "../api";
import { codeRule, passwordFieldReason } from "../validation";
import CodeField from "./CodeField.vue";
import GoogleError from "./GoogleError.vue";
import NoPasswordHint from "./NoPasswordHint.vue";

/**
 * Account → Google. Both directions re-confirm with password (+ code): linking gets the Google start URL
 * from the server (back to `/account?google=linked`), unlinking is a plain DELETE.
 */
const { t } = useI18n();
const route = useRoute();
const session = useSessionStore();
const toast = useToastStore();
const justLinked = route.query.google === "linked";
const confirming = ref<"link" | "unlink" | null>(null);
const form = reactive({ password: "", code: "" });
const { fieldErrors, error, submitting, submit } = useFormSubmit();

function open(mode: "link" | "unlink" | null) {
  confirming.value = mode;
  form.password = "";
  form.code = "";
  fieldErrors.value = {};
  error.value = null;
}

async function onSubmit() {
  const mode = confirming.value;
  const mfa = session.mfaEnabled;
  let url = "";
  const ok = await submit(
    () => collectErrors({ password: form.password === "" && "required", code: mfa && codeRule(form.code) }),
    async () => {
      const body = { password: form.password, ...(mfa ? { code: form.code.trim() } : {}) };
      if (mode === "link") url = (await authApi.linkGoogle(body)).url;
      else await authApi.unlinkGoogle(body);
    },
  );
  if (!ok) return;
  if (mode === "link") return leaveTo(url);
  open(null);
  session.patchUser({ googleLinked: false });
  toast.show(t("google.account.unlink.done"));
}
</script>

<template>
  <section v-if="session.context?.google || session.me?.user.googleLinked" class="card space-y-3" data-test="google-section">
    <h2 class="text-lg font-semibold">{{ t("google.account.title") }}</h2>
    <GoogleError />
    <p v-if="justLinked && session.me?.user.googleLinked" role="status" class="text-sm text-green-700 dark:text-green-400" data-test="google-linked-now">
      {{ t("google.account.linkedNow") }}
    </p>
    <p v-if="session.me?.user.googleLinked" class="text-sm" data-test="google-status">{{ t("google.account.linked") }}</p>
    <p v-else class="text-sm text-gray-600 dark:text-gray-400" data-test="google-status">{{ t("google.account.notLinked") }}</p>
    <NoPasswordHint v-if="!session.hasPassword" />
    <form v-else-if="confirming" class="space-y-4" novalidate :data-test="`google-${confirming}-form`" @submit.prevent="onSubmit">
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t(`google.account.${confirming}.intro`) }}</p>
      <FormField :label="t('mfa.password')" :for="`google-${confirming}-password`" :error="passwordFieldReason(fieldErrors.password)">
        <input
          :id="`google-${confirming}-password`"
          v-model="form.password"
          type="password"
          autocomplete="current-password"
          class="input"
          :class="{ 'input-error': fieldErrors.password }"
          :data-test="`google-${confirming}-password`"
        />
      </FormField>
      <CodeField v-if="session.mfaEnabled" :id="`google-${confirming}-code`" v-model="form.code" :error="fieldErrors.code" />
      <p v-if="error" role="alert" class="alert-error" :data-test="`google-${confirming}-error`">{{ error }}</p>
      <div class="flex gap-2">
        <button type="submit" class="btn" :class="confirming === 'unlink' ? 'btn-danger' : 'btn-primary'" :disabled="submitting" :data-test="`google-${confirming}-submit`">
          {{ t(`google.account.${confirming}.submit`) }}
        </button>
        <button type="button" class="btn" @click="open(null)">{{ t("common.cancel") }}</button>
      </div>
    </form>
    <button v-else-if="session.me?.user.googleLinked" type="button" class="btn" data-test="google-unlink-open" @click="open('unlink')">
      {{ t("google.account.unlink.open") }}
    </button>
    <button v-else-if="session.context?.google" type="button" class="btn" data-test="google-link-open" @click="open('link')">
      {{ t("google.account.link.open") }}
    </button>
  </section>
</template>
