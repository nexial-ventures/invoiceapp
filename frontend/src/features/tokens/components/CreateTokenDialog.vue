<script setup lang="ts">
import { computed, onMounted, reactive, ref, useTemplateRef } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import CodeField from "@/features/auth/components/CodeField.vue";
import { codeRule } from "@/features/auth/validation";
import { rolesUpTo } from "@/features/spaces/roles";
import type { Role } from "@/features/spaces/types";
import { collectErrors, isApiError, nullIfEmpty, textRule } from "@/lib/formErrors";
import { useSessionStore } from "@/stores/session";
import { useTokensStore } from "../store";
import McpConnectCard from "./McpConnectCard.vue";

const props = defineProps<{ role: Role }>();
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const store = useTokensStore();
const session = useSessionStore();

const roles = rolesUpTo(props.role);
// Least privilege by default; the user raises it deliberately.
const form = reactive<{ name: string; role: Role; expiresAt: string; code: string }>({ name: "", role: "accountant", expiresAt: "", code: "" });
const { fieldErrors, error, submitting, submit } = useFormSubmit();
/** The secret, held only in this dialog; gone when it closes. */
const secret = ref<string | null>(null);
const copied = ref(false);
const showMcp = ref(false);
/** 403 `mfa_required`: the space requires TOTP the user lacks. */
const mfaRequired = ref(false);
const accountUrl = computed(() => `${session.baseUrl.replace(/\/$/, "")}/account`);

/** Local "today" as `YYYY-MM-DD` (the date input's `min`). */
function today(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
const minDate = today();

async function onSubmit() {
  mfaRequired.value = false;
  const mfa = session.mfaEnabled;
  await submit(
    () =>
      collectErrors({
        name: textRule(form.name, { required: true, max: 100 }),
        expiresAt: form.expiresAt !== "" && form.expiresAt < minDate && "invalid",
        code: mfa && codeRule(form.code),
      }),
    async () => {
      const code = mfa ? { code: form.code.trim() } : {};
      try {
        const created = await store.create({ name: form.name.trim(), role: form.role, expiresAt: nullIfEmpty(form.expiresAt), ...code });
        secret.value = created.token;
      } catch (err) {
        mfaRequired.value = isApiError(err, 403, "mfa_required");
        throw err;
      }
    },
  );
}

async function copy() {
  if (!secret.value) return;
  try {
    await navigator.clipboard.writeText(secret.value);
    copied.value = true;
  } catch {
    // Clipboard blocked (permissions, insecure context): the token stays selectable in the field.
  }
}

const panel = useTemplateRef<HTMLElement>("panel");
onMounted(() => panel.value?.focus());
</script>

<template>
  <div class="fixed inset-0 z-40 flex items-start justify-center overflow-y-auto bg-black/40 p-4 sm:p-8" @click.self="secret || emit('close')">
    <div ref="panel" role="dialog" aria-modal="true" aria-labelledby="token-dialog-title" tabindex="-1" :class="['card w-full space-y-4 outline-none', showMcp ? 'max-w-2xl' : 'max-w-md']" data-test="token-dialog" @keydown.esc="emit('close')">
      <template v-if="secret">
        <h2 id="token-dialog-title" class="text-lg font-semibold">{{ t("tokens.created.title") }}</h2>
        <p class="rounded-md bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-950/50 dark:text-amber-300">{{ t("tokens.created.warning") }}</p>
        <FormField :label="t('tokens.created.label')" for="token-secret">
          <div class="flex gap-2">
            <input id="token-secret" :value="secret" readonly class="input font-mono" data-test="token-secret" @focus="($event.target as HTMLInputElement).select()" />
            <button type="button" class="btn" data-test="token-copy" @click="copy">{{ t("tokens.created.copy") }}</button>
          </div>
        </FormField>
        <p v-if="copied" role="status" class="text-sm text-green-700 dark:text-green-400">{{ t("tokens.created.copied") }}</p>
        <button type="button" class="btn" :aria-expanded="showMcp" aria-controls="token-mcp" data-test="token-mcp-toggle" @click="showMcp = !showMcp">
          {{ showMcp ? t("tokens.created.hideMcp") : t("tokens.created.showMcp") }}
        </button>
        <McpConnectCard v-if="showMcp" id="token-mcp" :token="secret" bare class="border-t border-gray-200 pt-4 dark:border-gray-700" />
        <div class="flex justify-end">
          <button type="button" class="btn btn-primary" data-test="token-done" @click="emit('close')">{{ t("tokens.created.done") }}</button>
        </div>
      </template>
      <form v-else class="space-y-4" novalidate @submit.prevent="onSubmit">
        <h2 id="token-dialog-title" class="text-lg font-semibold">{{ t("tokens.create.title") }}</h2>
        <FormField :label="t('tokens.create.name')" for="token-name" :error="fieldErrors.name">
          <input id="token-name" v-model="form.name" maxlength="100" class="input" :class="{ 'input-error': fieldErrors.name }" data-test="token-name" />
        </FormField>
        <FormField :label="t('tokens.create.role')" for="token-role" :error="fieldErrors.role">
          <select id="token-role" v-model="form.role" class="input" :class="{ 'input-error': fieldErrors.role }" data-test="token-role">
            <option v-for="r in roles" :key="r" :value="r">{{ t(`spaces.roles.${r}`) }}</option>
          </select>
        </FormField>
        <FormField :label="t('tokens.create.expires')" for="token-expires" :error="fieldErrors.expiresAt" :hint="t('tokens.create.expiresHint')">
          <input id="token-expires" v-model="form.expiresAt" type="date" :min="minDate" class="input" :class="{ 'input-error': fieldErrors.expiresAt }" data-test="token-expires" />
        </FormField>
        <CodeField v-if="session.mfaEnabled" id="token-code" v-model="form.code" :error="fieldErrors.code" />
        <div v-if="mfaRequired" role="alert" class="alert-error space-y-1" data-test="token-mfa-required">
          <p>{{ t("tokens.create.mfaRequired") }}</p>
          <a :href="accountUrl" class="font-medium underline" data-test="token-mfa-link">{{ t("tokens.create.mfaSetUp") }}</a>
        </div>
        <p v-else-if="error" role="alert" class="alert-error" data-test="token-error">{{ error }}</p>
        <div class="flex justify-end gap-2">
          <button type="button" class="btn" @click="emit('close')">{{ t("common.cancel") }}</button>
          <button type="submit" class="btn btn-primary" :disabled="submitting" data-test="token-create">
            {{ submitting ? t("tokens.create.submitting") : t("tokens.create.submit") }}
          </button>
        </div>
      </form>
    </div>
  </div>
</template>
