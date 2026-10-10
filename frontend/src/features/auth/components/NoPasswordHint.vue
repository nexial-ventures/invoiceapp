<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import { useSessionStore } from "@/stores/session";
import { authApi } from "../api";

/**
 * Stands in for a form that needs the user's password when the account has none (Google-only).
 * "Zapomenuté heslo" is a guest-only route, so a signed-in user gets the reset e-mail from here.
 */
const { t, locale } = useI18n();
const session = useSessionStore();
const { error, run } = useAction();
const sent = ref(false);

async function sendLink() {
  const email = session.me?.user.email;
  if (email && (await run(() => authApi.requestReset(email, locale.value)))) sent.value = true;
}
</script>

<template>
  <div class="space-y-2 text-sm" data-test="no-password-hint">
    <template v-if="session.me">
      <p class="text-amber-700 dark:text-amber-400">{{ t("google.noPassword.hint") }}</p>
      <p class="text-gray-600 dark:text-gray-400">{{ t("google.noPassword.howTo") }}</p>
      <p v-if="sent" role="status" data-test="no-password-sent">{{ t("google.noPassword.sent") }}</p>
      <button v-else type="button" class="btn" data-test="no-password-send" @click="sendLink">{{ t("google.noPassword.sendLink") }}</button>
      <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    </template>
    <template v-else>
      <p class="text-amber-700 dark:text-amber-400">{{ t("google.noPassword.guest") }}</p>
      <RouterLink :to="{ name: 'forgot' }" class="text-blue-600 hover:underline dark:text-blue-400">{{ t("auth.login.forgot") }}</RouterLink>
    </template>
  </div>
</template>
