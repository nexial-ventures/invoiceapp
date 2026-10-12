<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useAction } from "@/composables/useAction";
import { useDocumentStore } from "@/features/documents/store";
import { useEmailHistoryStore, useEmailSettingsStore } from "../store";
import SendEmailDialog from "./SendEmailDialog.vue";

/** "Send by e-mail" on an issued (non-draft, cancelled included) document of ours. */
const props = defineProps<{ documentId: string }>();

const { t } = useI18n();
const settings = useEmailSettingsStore();
const history = useEmailHistoryStore();
const documents = useDocumentStore();
const { error, run } = useAction();
const open = ref(false);

// Unknown status (still loading, or the request failed) leaves the button usable: the dialog reports the real error.
onMounted(() => void settings.ensureLoaded().catch(() => undefined));
const notConfigured = computed(() => settings.settings?.configured === false);
const disabled = computed(() => settings.settings?.disabled === true);
const unavailable = computed(() => notConfigured.value || disabled.value);

const reloadHistory = () => void run(() => history.load(props.documentId));

/** Closing (after a failed attempt too) refreshes the history: every attempt that reached SMTP is logged. */
function onClose(): void {
  open.value = false;
  reloadHistory();
}

/** The send set `sentAt` and logged an attempt: both the document and the history are stale. */
function onSent(): void {
  open.value = false;
  void run(async () => {
    await Promise.all([documents.load(props.documentId), history.load(props.documentId)]);
  });
}
</script>

<template>
  <span class="inline-flex flex-wrap items-center gap-2">
    <button
      type="button"
      class="btn"
      :disabled="unavailable"
      :aria-describedby="unavailable ? 'email-not-configured-hint' : undefined"
      data-test="send-email"
      @click="open = true"
    >
      {{ t("email.send.button") }}
    </button>
    <span v-if="unavailable" id="email-not-configured-hint" class="text-xs text-gray-600 dark:text-gray-400" data-test="send-email-hint">
      {{ disabled ? t("email.send.disabledHint") : t("email.send.notConfiguredHint") }}
    </span>
    <span v-if="error" role="alert" class="alert-error">{{ error }}</span>
    <SendEmailDialog v-if="open" :document-id="documentId" @close="onClose" @sent="onSent" @attempted="reloadHistory" />
  </span>
</template>
