<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import LeaveSpaceSection from "@/features/members/components/LeaveSpaceSection.vue";
import { useSessionStore } from "@/stores/session";
import { useToastStore } from "@/stores/toast";
import { authApi } from "../api";
import GoogleSection from "../components/GoogleSection.vue";
import MfaSection from "../components/MfaSection.vue";
import PasswordSection from "../components/PasswordSection.vue";

const { t } = useI18n();
const session = useSessionStore();
const toast = useToastStore();
const revoke = useAction();

async function revokeOthers() {
  if (!window.confirm(t("account.sessions.confirm"))) return;
  if (await revoke.run(authApi.revokeOthers)) toast.show(t("account.sessions.revoked"));
}
</script>

<template>
  <section class="max-w-xl space-y-4">
    <h1 class="text-2xl font-semibold">{{ t("account.title") }}</h1>

    <dl v-if="session.me" class="card grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
      <dt class="text-gray-500">{{ t("account.name") }}</dt>
      <dd data-test="account-name">{{ session.me.user.displayName }}</dd>
      <dt class="text-gray-500">{{ t("account.email") }}</dt>
      <dd data-test="account-email">{{ session.me.user.email }}</dd>
    </dl>

    <PasswordSection />

    <GoogleSection />

    <MfaSection />

    <section class="card space-y-3">
      <h2 class="text-lg font-semibold">{{ t("account.sessions.title") }}</h2>
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("account.sessions.intro") }}</p>
      <p v-if="revoke.error.value" role="alert" class="alert-error">{{ revoke.error.value }}</p>
      <button type="button" class="btn" data-test="revoke-others" @click="revokeOthers">{{ t("account.sessions.revoke") }}</button>
    </section>

    <LeaveSpaceSection v-if="session.role" />
  </section>
</template>
