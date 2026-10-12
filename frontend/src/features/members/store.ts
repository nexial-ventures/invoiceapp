import { defineStore } from "pinia";
import { computed, ref } from "vue";
import type { Role } from "@/features/spaces/types";
import { membersApi } from "./api";
import type { Invite, InviteBody, Member, SentInvite } from "./types";

/** Settings → Členové: the space's members and pending invitations. */
export const useMembersStore = defineStore("members", () => {
  const members = ref<Member[]>([]);
  const invites = ref<Invite[]>([]);
  const loaded = ref(false);
  const ownersCount = computed(() => members.value.filter((m) => m.role === "owner").length);

  async function load(): Promise<void> {
    [members.value, invites.value] = await Promise.all([membersApi.list(), membersApi.invites()]);
    loaded.value = true;
  }

  async function setRole(userId: string, role: Role): Promise<void> {
    const updated = await membersApi.setRole(userId, role);
    members.value = members.value.map((m) => (m.userId === userId ? updated : m));
  }

  async function remove(userId: string): Promise<void> {
    await membersApi.remove(userId);
    members.value = members.value.filter((m) => m.userId !== userId);
  }

  /** The link in the result is shown once by the caller and never kept here. */
  async function invite(body: InviteBody): Promise<SentInvite> {
    const sent = await membersApi.invite(body);
    // A pending invitation for the same e-mail is replaced by the server.
    invites.value = [toRow(sent), ...invites.value.filter((i) => i.email !== sent.email)];
    return sent;
  }

  async function resend(id: string): Promise<SentInvite> {
    const sent = await membersApi.resend(id);
    invites.value = invites.value.map((i) => (i.id === id ? toRow(sent) : i));
    return sent;
  }

  async function revoke(id: string): Promise<void> {
    await membersApi.revoke(id);
    invites.value = invites.value.filter((i) => i.id !== id);
  }

  return { members, invites, loaded, ownersCount, load, setRole, remove, invite, resend, revoke };
});

function toRow({ url: _url, emailSent: _sent, emailDisabled: _disabled, ...row }: SentInvite): Invite {
  return row;
}
