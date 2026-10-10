<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { ApiError } from "@/api/client";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import AuthCard from "@/features/auth/components/AuthCard.vue";
import CodeField from "@/features/auth/components/CodeField.vue";
import GoogleButton from "@/features/auth/components/GoogleButton.vue";
import GoogleError from "@/features/auth/components/GoogleError.vue";
import NoPasswordHint from "@/features/auth/components/NoPasswordHint.vue";
import { codeRule, passwordFieldReason, passwordRule } from "@/features/auth/validation";
import { collectErrors, isApiError, textRule } from "@/lib/formErrors";
import { useSessionStore } from "@/stores/session";
import { membersApi } from "../api";
import InviteMfaRequired from "../components/InviteMfaRequired.vue";
import type { InviteInfo } from "../types";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const session = useSessionStore();
const token = typeof route.query.token === "string" ? route.query.token : "";
const info = ref<InviteInfo | null>(null);
const form = reactive({ displayName: "", password: "", code: "" });
const { fieldErrors, error, submitting, submit, showError } = useFormSubmit();
const lookupNotFound = ref(false);
/** 403 `mfa_required`: the space requires TOTP the account lacks; the invitation stays valid. */
const mfaOutcome = ref<"existing" | "created" | null>(null);
/**
 * The lookup does not disclose whether the account has TOTP, so the code is required only once the
 * server asked for it (422 `code`). In a space requiring TOTP the field is offered up front but left
 * optional: an account without TOTP must reach the 403 `mfa_required` explanation.
 */
const codeAsked = ref(false);
const requireCode = computed(() => !!info.value?.accountExists && (session.mfaEnabled || codeAsked.value));
const showCode = computed(() => requireCode.value || (!!info.value?.accountExists && info.value.requireMfa));
/** The invited account exists but has no password (Google-only): accepting needs Google or a password set first. */
const noPassword = computed(
  () => fieldErrors.value.password === "not_set" || (!!info.value?.accountExists && session.me?.user.email === info.value.email && !session.hasPassword),
);
/** 404 on the lookup or 422 `token` on accept: used, replaced, expired or mangled. */
const invalid = computed(() => !token || fieldErrors.value.token !== undefined || lookupNotFound.value);

onMounted(async () => {
  if (!token) return;
  try {
    info.value = await membersApi.inviteInfo(token);
  } catch (err) {
    if (err instanceof ApiError && err.status === 404) lookupNotFound.value = true;
    else showError(err);
  }
});

function validate() {
  const exists = info.value?.accountExists;
  return collectErrors({
    password: exists ? form.password === "" && "required" : passwordRule(form.password),
    displayName: !exists && textRule(form.displayName, { required: true, max: 100 }),
    code: requireCode.value && codeRule(form.code),
  });
}

async function onSubmit() {
  const exists = info.value?.accountExists;
  const code = form.code.trim();
  const extra = exists ? (showCode.value && code !== "" ? { code } : {}) : { displayName: form.displayName.trim() };
  const ok = await submit(validate, async () => {
    try {
      await membersApi.accept({ token, password: form.password, ...extra });
    } catch (err) {
      if (isApiError(err, 403, "mfa_required")) mfaOutcome.value = err.detail === "account_created" ? "created" : "existing";
      if (err instanceof ApiError && err.fields.code) codeAsked.value = true;
      throw err;
    }
  });
  if (!ok) return;
  // The accept set this host's session cookie; pick up the user and the new role.
  await session.loadMe();
  await router.replace({ name: "home" });
}
</script>

<template>
  <AuthCard :title="t('members.accept.title')">
    <template v-if="invalid">
      <p role="alert" class="alert-error" data-test="accept-invalid">{{ t("members.accept.invalid") }}</p>
      <a :href="session.baseUrl" class="btn w-full" data-test="accept-base-link">{{ t("members.accept.toBase") }}</a>
    </template>
    <p v-else-if="!info && !error" class="text-sm text-gray-500">{{ t("common.loading") }}</p>
    <p v-else-if="!info" role="alert" class="alert-error" data-test="accept-error">{{ error }}</p>
    <InviteMfaRequired v-else-if="mfaOutcome" :outcome="mfaOutcome" :space="info.space.name" />
    <template v-else>
      <p class="text-sm" data-test="accept-intro">
        {{ t("members.accept.intro", { name: info.space.name, role: t(`spaces.roles.${info.role}`) }) }}
      </p>
      <GoogleError />
      <NoPasswordHint v-if="noPassword" />
      <form v-else class="space-y-4" novalidate @submit.prevent="onSubmit">
        <p class="text-sm text-gray-600 dark:text-gray-400">
          {{ info.accountExists ? t("members.accept.existing") : t("members.accept.new") }}
        </p>
        <p v-if="info.requireMfa" class="text-sm text-amber-700 dark:text-amber-400" data-test="accept-require-mfa">
          {{ info.accountExists ? t("members.accept.requireMfaExisting") : t("members.accept.requireMfaNew") }}
        </p>
        <FormField :label="t('auth.email')" for="accept-email">
          <input id="accept-email" :value="info.email" type="email" autocomplete="username" readonly class="input" data-test="accept-email" />
        </FormField>
        <FormField v-if="!info.accountExists" :label="t('auth.register.displayName')" for="accept-name" :error="fieldErrors.displayName">
          <input id="accept-name" v-model="form.displayName" autocomplete="name" maxlength="100" class="input" :class="{ 'input-error': fieldErrors.displayName }" data-test="accept-name" />
        </FormField>
        <FormField
          :label="t('auth.password')"
          for="accept-password"
          :error="passwordFieldReason(fieldErrors.password)"
          :hint="info.accountExists ? undefined : t('auth.passwordHint')"
        >
          <input
            id="accept-password"
            v-model="form.password"
            type="password"
            :autocomplete="info.accountExists ? 'current-password' : 'new-password'"
            class="input"
            :class="{ 'input-error': fieldErrors.password }"
            data-test="accept-password"
          />
        </FormField>
        <CodeField v-if="showCode" id="accept-code" v-model="form.code" :error="fieldErrors.code" />
        <p v-if="error" role="alert" class="alert-error" data-test="accept-error">{{ error }}</p>
        <button type="submit" class="btn btn-primary w-full" :disabled="submitting" data-test="accept-submit">
          {{ submitting ? t("members.accept.submitting") : info.accountExists ? t("members.accept.submitExisting") : t("members.accept.submitNew") }}
        </button>
      </form>
      <GoogleButton purpose="invite" :invite="token" :label="t('google.button.invite')" divider />
    </template>
  </AuthCard>
</template>
