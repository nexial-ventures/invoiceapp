<script setup lang="ts">
import { computed, onMounted, reactive, ref, useTemplateRef, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ApiError } from "@/api/client";
import type { DocLocale, FieldErrors } from "@/api/types";
import ErrorDetail from "@/components/ErrorDetail.vue";
import FormField from "@/components/form/FormField.vue";
import { LOCALES } from "@/i18n";
import { collectErrors, errorDetailOf, fieldErrorsOf, textRule } from "@/lib/formErrors";
import { useToastStore } from "@/stores/toast";
import { emailApi } from "../api";
import { recipientErrors, withAddress } from "../recipients";
import { useEmailSettingsStore } from "../store";
import { useEmailErrorText } from "../useEmailErrorText";
import { localizeTemplateDetail } from "../templateDetail";
import type { EmailPrefill, RecipientField, SendEmailInput } from "../types";
import RecipientInput from "./RecipientInput.vue";

const props = defineProps<{ documentId: string }>();
/** `attempted`: the send reached SMTP and failed — the server logged it, so the history is stale. */
const emit = defineEmits<{ close: []; sent: []; attempted: [] }>();

const { t } = useI18n();
const errorText = useEmailErrorText();
const toast = useToastStore();
const settings = useEmailSettingsStore();

const prefill = ref<EmailPrefill | null>(null);
const locale = ref<DocLocale>("cs");
const form = reactive<SendEmailInput>({ to: [], cc: [], bcc: [], subject: "", body: "", attachPdf: false, attachIsdoc: true });
const loading = ref(false);
const sending = ref(false);
const error = ref<string | null>(null);
const detail = ref<string | null>(null);
const fieldErrors = ref<FieldErrors>({});

/** The stored template fails on this document's data (prefill): only Settings can fix it. */
const templateBroken = ref(false);

function fail(err: unknown): void {
  const code = err instanceof ApiError ? err.code : null;
  const raw = errorDetailOf(err);
  templateBroken.value = code === "template_invalid";
  if (templateBroken.value) {
    // Its 422 carries a `fields` map too, but those name the template, not this form.
    fieldErrors.value = {};
    error.value = errorText(err);
    const localized = raw ? localizeTemplateDetail(raw, t) : null;
    detail.value = localized ? [localized.text, localized.hint].filter(Boolean).join("\n") : null;
    return;
  }
  const fields = fieldErrorsOf(err);
  fieldErrors.value = fields ?? {};
  error.value = fields ? t("errors.validation") : errorText(err);
  detail.value = raw;
  if (code === "smtp_failed") emit("attempted");
}

function clearErrors(): void {
  templateBroken.value = false;
  error.value = null;
  detail.value = null;
  fieldErrors.value = {};
}

/** First load takes everything; a locale switch only replaces the rendered text (recipients stay as edited). */
async function load(next?: DocLocale): Promise<void> {
  loading.value = true;
  clearErrors();
  try {
    const p = await emailApi.prefill(props.documentId, next);
    if (!prefill.value) {
      Object.assign(form, { to: p.to, cc: p.cc, bcc: p.bcc, attachPdf: p.attachments.pdf.available });
    }
    form.subject = p.subject;
    form.body = p.body;
    form.attachPdf &&= p.attachments.pdf.available;
    prefill.value = p;
    locale.value = p.locale;
  } catch (err) {
    fail(err);
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  void settings.ensureLoaded().catch(() => undefined);
  void load();
});

const edited = computed(() => !!prefill.value && (form.subject !== prefill.value.subject || form.body !== prefill.value.body));

function switchLocale(e: Event): void {
  const select = e.target as HTMLSelectElement;
  const next = select.value as DocLocale;
  if (edited.value && !window.confirm(t("email.send.confirmLocale"))) {
    select.value = locale.value;
    return;
  }
  void load(next);
}

const me = computed(() => settings.settings?.replyTo ?? null);
const bccToMe = computed({
  get: () => !!me.value && form.bcc.some((a) => a.toLowerCase() === me.value!.toLowerCase()),
  set: (on: boolean) => {
    if (me.value) form.bcc = withAddress(form.bcc, me.value, on);
  },
});

const fields: RecipientField[] = ["to", "cc", "bcc"];
const listErrors = computed(() => Object.fromEntries(fields.map((f) => [f, recipientErrors(fieldErrors.value, f)])));

// Server reasons point at list indices; once a list changes they would mark the wrong chip.
for (const f of fields) {
  watch(
    () => form[f].length,
    () => {
      fieldErrors.value = Object.fromEntries(Object.entries(fieldErrors.value).filter(([k]) => k !== f && !k.startsWith(`${f}.`)));
    },
  );
}

const configured = computed(() => prefill.value?.configured ?? false);
const disabled = computed(() => prefill.value?.disabled ?? false);
const canSend = computed(() => configured.value && !disabled.value && !sending.value && !loading.value);

async function send(): Promise<void> {
  if (!canSend.value) return;
  clearErrors();
  fieldErrors.value = collectErrors({
    to: form.to.length === 0 && "required",
    subject: textRule(form.subject, { required: true, max: 500 }),
  });
  if (Object.keys(fieldErrors.value).length > 0) {
    error.value = t("errors.validation");
    return;
  }
  sending.value = true;
  try {
    await emailApi.send(props.documentId, { ...form });
    toast.show(t("email.send.sent", { to: form.to.join(", ") }));
    emit("sent");
  } catch (err) {
    fail(err);
  } finally {
    sending.value = false;
  }
}

const panel = useTemplateRef<HTMLElement>("panel");
onMounted(() => panel.value?.focus());
</script>

<template>
  <div class="fixed inset-0 z-40 flex items-start justify-center overflow-y-auto bg-black/40 p-4 sm:p-8" @click.self="emit('close')">
    <div
      ref="panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="send-email-title"
      tabindex="-1"
      class="card w-full max-w-2xl space-y-4 outline-none"
      data-test="send-email-dialog"
      @keydown.esc="emit('close')"
    >
      <div class="flex items-center justify-between gap-2">
        <h2 id="send-email-title" class="text-lg font-semibold">{{ t("email.send.title") }}</h2>
        <label class="flex items-center gap-2 text-sm">
          {{ t("locale.label") }}
          <select class="input !w-auto !py-1" :value="locale" :disabled="loading || sending" data-test="email-locale" @change="switchLocale">
            <option v-for="code in LOCALES" :key="code" :value="code">{{ t(`locale.names.${code}`) }}</option>
          </select>
        </label>
      </div>

      <p v-if="prefill && disabled" class="alert-error" data-test="email-disabled">{{ t("email.send.disabled") }}</p>
      <p v-else-if="prefill && !configured" class="alert-error" data-test="email-not-configured">{{ t("email.send.notConfigured") }}</p>
      <p v-if="loading && !prefill" class="text-sm text-gray-500">{{ t("common.loading") }}</p>

      <form v-if="prefill" class="space-y-3" novalidate @submit.prevent="send">
        <FormField v-for="f in fields" :key="f" :label="t(`email.send.${f}`)" :for="`email-${f}`" :error="listErrors[f]!.list ?? undefined">
          <RecipientInput :id="`email-${f}`" v-model="form[f]" :invalid="listErrors[f]!.items" :list-error="!!listErrors[f]!.list" />
        </FormField>
        <p class="text-xs text-gray-500 dark:text-gray-400">{{ t("email.send.recipientsHint") }}</p>
        <label v-if="me" class="flex items-center gap-2 text-sm">
          <input v-model="bccToMe" type="checkbox" data-test="bcc-to-me" />
          {{ t("email.send.bccToMe", { address: me }) }}
        </label>

        <FormField :label="t('email.send.subject')" for="email-subject" :error="fieldErrors.subject">
          <input id="email-subject" v-model="form.subject" class="input" :class="{ 'input-error': fieldErrors.subject }" data-test="email-subject" />
        </FormField>
        <FormField :label="t('email.send.body')" for="email-body" :error="fieldErrors.body">
          <textarea id="email-body" v-model="form.body" rows="12" class="input font-mono" :class="{ 'input-error': fieldErrors.body }" data-test="email-body" />
        </FormField>

        <fieldset class="space-y-1">
          <legend class="text-sm font-medium">{{ t("email.send.attachments") }}</legend>
          <label class="flex items-center gap-2 text-sm">
            <input v-model="form.attachPdf" type="checkbox" :disabled="!prefill.attachments.pdf.available" data-test="attach-pdf" />
            <span class="font-mono">{{ prefill.attachments.pdf.filename }}</span>
            <span v-if="!prefill.attachments.pdf.available" class="text-xs text-gray-500">{{ t("email.send.pdfUnavailable") }}</span>
          </label>
          <p v-if="fieldErrors.attachPdf" class="text-xs text-red-600 dark:text-red-400" data-test="field-error">{{ t("email.send.pdfUnavailable") }}</p>
          <label class="flex items-center gap-2 text-sm">
            <input v-model="form.attachIsdoc" type="checkbox" data-test="attach-isdoc" />
            <span class="font-mono">{{ prefill.attachments.isdoc.filename }}</span>
          </label>
        </fieldset>
      </form>

      <div v-if="error" role="alert" class="alert-error" data-test="email-error">
        <p>{{ error }}</p>
        <i18n-t v-if="templateBroken" keypath="email.send.templateFix" tag="p" data-test="template-fix">
          <template #link>
            <RouterLink :to="{ name: 'settings-email' }" class="underline">{{ t("email.send.templateFixLink") }}</RouterLink>
          </template>
        </i18n-t>
        <ErrorDetail :detail="detail" open />
      </div>

      <div class="flex justify-end gap-2">
        <button type="button" class="btn" data-test="email-cancel" @click="emit('close')">{{ t("common.cancel") }}</button>
        <button type="button" class="btn btn-primary" :disabled="!canSend" data-test="email-send" @click="send">
          {{ sending ? t("email.send.sending") : t("email.send.submit") }}
        </button>
      </div>
    </div>
  </div>
</template>
