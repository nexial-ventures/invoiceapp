<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { emailRule } from "@/features/auth/validation";
import { useEmailErrorText } from "@/features/email/useEmailErrorText";
import { formatDate } from "@/features/documents/format";
import type { Role } from "@/features/spaces/types";
import { collectErrors } from "@/lib/formErrors";
import { useSessionStore } from "@/stores/session";
import { useToastStore } from "@/stores/toast";
import { grantableRoles } from "../roles";
import { useMembersStore } from "../store";
import type { Invite, SentInvite } from "../types";
import { useMemberAction } from "../useMemberAction";
import InviteLink from "./InviteLink.vue";

const { t, locale } = useI18n();
const session = useSessionStore();
const store = useMembersStore();
const toast = useToastStore();
const roles = computed(() => grantableRoles(session.role));
const form = reactive<{ email: string; role: Role }>({ email: "", role: "member" });
const emailErrorText = useEmailErrorText();
const { fieldErrors, error, submitting, submit } = useFormSubmit(emailErrorText);
const action = useMemberAction(emailErrorText);
/** The last invitation sent or resent: its link is shown once, never stored. */
const sent = ref<SentInvite | null>(null);

async function onSubmit() {
  await submit(
    () => collectErrors({ email: emailRule(form.email) }),
    async () => {
      sent.value = await store.invite({ email: form.email.trim(), role: form.role, locale: locale.value });
      form.email = "";
    },
  );
}

async function resend(invite: Invite) {
  await action.run(async () => (sent.value = await store.resend(invite.id)));
}

async function revoke(invite: Invite) {
  if (!window.confirm(t("members.pending.confirmRevoke", { email: invite.email }))) return;
  if (!(await action.run(() => store.revoke(invite.id)))) return;
  if (sent.value?.id === invite.id) sent.value = null;
  toast.show(t("members.pending.revoked"));
}
</script>

<template>
  <section class="card space-y-4" data-test="invites">
    <h2 class="text-lg font-semibold">{{ t("members.invite.title") }}</h2>
    <form class="grid gap-4 sm:grid-cols-[1fr_auto_auto] sm:items-start" novalidate @submit.prevent="onSubmit">
      <FormField :label="t('members.invite.email')" for="invite-email" :error="fieldErrors.email">
        <input id="invite-email" v-model="form.email" type="email" autocomplete="off" class="input" :class="{ 'input-error': fieldErrors.email }" data-test="invite-email" />
      </FormField>
      <FormField :label="t('members.invite.role')" for="invite-role" :error="fieldErrors.role">
        <select id="invite-role" v-model="form.role" class="input" :class="{ 'input-error': fieldErrors.role }" data-test="invite-role">
          <option v-for="r in roles" :key="r" :value="r">{{ t(`spaces.roles.${r}`) }}</option>
        </select>
      </FormField>
      <button type="submit" class="btn btn-primary sm:mt-6" :disabled="submitting" data-test="invite-submit">
        {{ submitting ? t("members.invite.submitting") : t("members.invite.submit") }}
      </button>
    </form>
    <p v-if="error" role="alert" class="alert-error" data-test="invite-error">{{ error }}</p>
    <InviteLink v-if="sent" :sent="sent" />

    <h3 class="font-semibold">{{ t("members.pending.title") }}</h3>
    <p v-if="action.error.value" role="alert" class="alert-error" data-test="invites-error">{{ action.error.value }}</p>
    <p v-if="store.invites.length === 0" class="text-sm text-gray-500" data-test="invites-empty">{{ t("members.pending.empty") }}</p>
    <div v-else class="overflow-x-auto">
      <table class="table">
        <thead>
          <tr>
            <th>{{ t("members.columns.email") }}</th>
            <th>{{ t("members.columns.role") }}</th>
            <th>{{ t("members.pending.invitedBy") }}</th>
            <th>{{ t("members.pending.expires") }}</th>
            <th><span class="sr-only">{{ t("common.actions") }}</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="invite in store.invites" :key="invite.id" data-test="invite-row">
            <td>{{ invite.email }}</td>
            <td>{{ t(`spaces.roles.${invite.role}`) }}</td>
            <td>{{ invite.invitedBy.displayName }}</td>
            <td class="whitespace-nowrap">{{ formatDate(invite.expiresAt, locale) }}</td>
            <!-- An admin cannot touch an owner invitation (403): no actions on that row. -->
            <td class="space-x-1 text-right whitespace-nowrap">
              <template v-if="roles.includes(invite.role)">
                <button type="button" class="btn btn-sm" data-test="invite-resend" @click="resend(invite)">{{ t("members.pending.resend") }}</button>
                <button type="button" class="btn btn-sm btn-danger" data-test="invite-revoke" @click="revoke(invite)">{{ t("members.pending.revoke") }}</button>
              </template>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>
