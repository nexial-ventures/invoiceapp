<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { useAction } from "@/composables/useAction";
import { useSessionStore } from "@/stores/session";
import { useToastStore } from "@/stores/toast";
import { useMfaStore } from "../mfaStore";
import MfaConfirmForm from "./MfaConfirmForm.vue";
import MfaEnableFlow from "./MfaEnableFlow.vue";
import NoPasswordHint from "./NoPasswordHint.vue";
import RecoveryCodesPanel from "./RecoveryCodesPanel.vue";

/** Account → "Dvoufázové ověření": status, enrolment, recovery codes, turning it off. */
const { t } = useI18n();
const router = useRouter();
const session = useSessionStore();
const toast = useToastStore();
const store = useMfaStore();
const { error, run } = useAction();
const panel = ref<"enable" | "regenerate" | "disable" | null>(null);
/** Fresh recovery codes, held only until the user confirms they saved them. */
const codes = ref<string[] | null>(null);

onMounted(() => void run(store.load));

function showCodes(next: string[] | null) {
  panel.value = null;
  codes.value = next;
}

async function onDisabled() {
  const here = session.context?.space?.slug;
  // The server has just ended this host's session if this space requires TOTP.
  const signedOut = here !== undefined && store.status?.requiredBy.some((s) => s.slug === here);
  panel.value = null;
  toast.show(t("mfa.disable.done"));
  if (signedOut) {
    session.clear();
    await router.replace({ name: "login" });
  }
}
</script>

<template>
  <section class="card space-y-3" data-test="mfa-section">
    <h2 class="text-lg font-semibold">{{ t("mfa.title") }}</h2>
    <p v-if="error" role="alert" class="alert-error" data-test="mfa-error">{{ error }}</p>
    <p v-else-if="!store.status" class="text-sm text-gray-500">{{ t("common.loading") }}</p>
    <RecoveryCodesPanel v-else-if="codes" :codes="codes" @close="codes = null" />
    <MfaEnableFlow v-else-if="panel === 'enable'" @enabled="showCodes" @cancel="panel = null" />
    <MfaConfirmForm v-else-if="panel === 'regenerate'" kind="regenerate" @done="showCodes" @cancel="panel = null" />
    <MfaConfirmForm v-else-if="panel === 'disable'" kind="disable" @done="onDisabled" @cancel="panel = null" />
    <template v-else-if="store.status.enabled">
      <p class="text-sm" data-test="mfa-status">{{ t("mfa.on", { count: store.status.recoveryCodesLeft }) }}</p>
      <p v-if="store.status.recoveryCodesLeft <= 3" class="text-sm text-amber-700 dark:text-amber-400" data-test="mfa-codes-low">{{ t("mfa.codesLow") }}</p>
      <NoPasswordHint v-if="!session.hasPassword" />
      <div v-else class="flex flex-wrap gap-2">
        <button type="button" class="btn" data-test="mfa-open-regenerate" @click="panel = 'regenerate'">{{ t("mfa.regenerate.open") }}</button>
        <button type="button" class="btn btn-danger" data-test="mfa-open-disable" @click="panel = 'disable'">{{ t("mfa.disable.open") }}</button>
      </div>
    </template>
    <template v-else>
      <p class="text-sm text-gray-600 dark:text-gray-400" data-test="mfa-status">{{ t("mfa.off") }}</p>
      <p v-if="store.status.requiredBy.length" class="text-sm text-amber-700 dark:text-amber-400" data-test="mfa-required-hint">
        {{ t("mfa.requiredHint", { spaces: store.status.requiredBy.map((s) => s.name).join(", ") }) }}
      </p>
      <NoPasswordHint v-if="!session.hasPassword" />
      <button v-else type="button" class="btn btn-primary" data-test="mfa-open-enable" @click="panel = 'enable'">{{ t("mfa.enable.open") }}</button>
    </template>
  </section>
</template>
