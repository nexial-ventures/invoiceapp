<script setup lang="ts">
import { onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { formatDate } from "@/lib/format";
import type { Role } from "@/features/spaces/types";
import { useSessionStore } from "@/stores/session";
import { useToastStore } from "@/stores/toast";
import InvitesSection from "../components/InvitesSection.vue";
import { assignableRoles } from "../roles";
import { useMembersStore } from "../store";
import type { Member } from "../types";
import { useMemberAction } from "../useMemberAction";

const { t, locale } = useI18n();
const session = useSessionStore();
const store = useMembersStore();
const toast = useToastStore();
const { error, run } = useMemberAction();

onMounted(() => void run(store.load));

const rolesFor = (member: Member) => assignableRoles(session.role, member.role, store.ownersCount);

async function changeRole(member: Member, event: Event) {
  const select = event.target as HTMLSelectElement;
  const role = select.value as Role;
  // Stepping down may take away the rights this very page needs.
  const ok = (!member.isSelf || window.confirm(t("members.confirmOwnRole"))) && (await run(() => store.setRole(member.userId, role)));
  if (!ok) {
    select.value = member.role;
    return;
  }
  if (member.isSelf) await session.loadMe();
  toast.show(t("members.roleChanged"));
}

async function remove(member: Member) {
  if (!window.confirm(t("members.confirmRemove", { name: member.displayName }))) return;
  if (await run(() => store.remove(member.userId))) toast.show(t("members.removed"));
}
</script>

<template>
  <div class="space-y-6">
    <section class="card space-y-4">
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("members.intro") }}</p>
      <p v-if="error" role="alert" class="alert-error" data-test="members-error">{{ error }}</p>
      <p v-if="!store.loaded && !error" class="text-sm text-gray-500">{{ t("common.loading") }}</p>
      <div v-else-if="store.loaded" class="overflow-x-auto">
        <table class="table">
          <thead>
            <tr>
              <th>{{ t("members.columns.name") }}</th>
              <th>{{ t("members.columns.email") }}</th>
              <th>{{ t("members.columns.role") }}</th>
              <th>{{ t("members.columns.mfa") }}</th>
              <th>{{ t("members.columns.joined") }}</th>
              <th><span class="sr-only">{{ t("common.actions") }}</span></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="member in store.members" :key="member.userId" data-test="member-row">
              <td class="font-medium">
                {{ member.displayName }}
                <span v-if="member.isSelf" class="text-xs font-normal text-gray-500" data-test="member-self">{{ t("members.you") }}</span>
              </td>
              <td>{{ member.email }}</td>
              <td>
                <select
                  v-if="rolesFor(member).length > 0"
                  :value="member.role"
                  :aria-label="t('members.columns.role')"
                  class="input py-1"
                  data-test="member-role"
                  @change="changeRole(member, $event)"
                >
                  <option v-for="r in rolesFor(member)" :key="r" :value="r">{{ t(`spaces.roles.${r}`) }}</option>
                </select>
                <span v-else data-test="member-role-text">{{ t(`spaces.roles.${member.role}`) }}</span>
              </td>
              <td data-test="member-mfa">
                <span v-if="member.mfaEnabled" class="badge">{{ t("members.mfa.on") }}</span>
                <span v-else class="text-gray-500">{{ t("members.mfa.off") }}</span>
              </td>
              <td class="whitespace-nowrap">{{ formatDate(member.joinedAt, locale) }}</td>
              <td class="text-right">
                <!-- Out of reach = not removable either; oneself leaves from the account page. -->
                <button v-if="!member.isSelf && rolesFor(member).length > 0" type="button" class="btn btn-sm btn-danger" data-test="member-remove" @click="remove(member)">
                  {{ t("members.remove") }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
    <InvitesSection v-if="store.loaded" />
  </div>
</template>
