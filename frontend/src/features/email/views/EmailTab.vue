<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { DocLocale } from "@/api/types";
import ErrorDetail from "@/components/ErrorDetail.vue";
import FormField from "@/components/form/FormField.vue";
import { useAction } from "@/composables/useAction";
import { errorDetailOf, fieldErrorsOf, nullIfEmpty } from "@/lib/formErrors";
import { useToastStore } from "@/stores/toast";
import { emailApi } from "../api";
import EmailTemplateEditor from "../components/EmailTemplateEditor.vue";
import TemplateVariables from "../components/TemplateVariables.vue";
import { useEmailSettingsStore } from "../store";
import { useEmailErrorText } from "../useEmailErrorText";

const { t } = useI18n();
const store = useEmailSettingsStore();
const toast = useToastStore();
const { error: loadError, run } = useAction();
const errorText = useEmailErrorText();

onMounted(() => void run(() => Promise.all([store.load(), store.loadTemplates()]).then(() => undefined)));

const active = ref<DocLocale>("cs");

const testTo = ref("");
const testing = ref(false);
const testError = ref<string | null>(null);
const testDetail = ref<string | null>(null);
const testField = ref<string | undefined>();

async function sendTest(): Promise<void> {
  if (testing.value) return;
  testing.value = true;
  testError.value = null;
  testDetail.value = null;
  testField.value = undefined;
  try {
    await emailApi.test(nullIfEmpty(testTo.value));
    toast.show(t("email.settings.testSent"));
  } catch (err) {
    testField.value = fieldErrorsOf(err)?.to;
    testError.value = testField.value ? t("errors.validation") : errorText(err);
    testDetail.value = errorDetailOf(err);
  } finally {
    testing.value = false;
  }
}
</script>

<template>
  <div class="space-y-4">
    <p v-if="loadError" role="alert" class="alert-error">{{ loadError }}</p>

    <section v-if="store.settings" class="card space-y-3" data-test="email-status">
      <h2 class="text-lg font-semibold">{{ t("email.settings.status") }}</h2>
      <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
        <dt class="text-gray-600 dark:text-gray-400">{{ t("email.settings.configured") }}</dt>
        <dd data-test="email-configured">
          <span v-if="store.settings.configured" class="badge !bg-green-100 !text-green-800 dark:!bg-green-900/50 dark:!text-green-300">{{ t("common.yes") }}</span>
          <span v-else class="badge !bg-red-100 !text-red-800 dark:!bg-red-900/50 dark:!text-red-300">{{ t("common.no") }}</span>
        </dd>
        <dt class="text-gray-600 dark:text-gray-400">{{ t("email.settings.from") }}</dt>
        <dd class="break-all" data-test="email-from">{{ store.settings.from ?? t("common.notSet") }}</dd>
        <dt class="text-gray-600 dark:text-gray-400">{{ t("email.settings.replyTo") }}</dt>
        <dd class="break-all" data-test="email-reply-to">{{ store.settings.replyTo ?? t("email.settings.replyToMissing") }}</dd>
      </dl>
      <p v-if="store.settings.disabled" class="alert-error" data-test="email-disabled">{{ t("email.settings.disabled") }}</p>
      <p v-if="!store.settings.configured" class="text-sm text-gray-600 dark:text-gray-400" data-test="email-config-help">{{ t("email.settings.configHelp") }}</p>

      <form class="flex flex-wrap items-end gap-2" novalidate @submit.prevent="sendTest">
        <div class="min-w-60 flex-1">
          <FormField :label="t('email.settings.testTo')" for="email-test-to" :error="testField" :hint="t('email.settings.testToHint')">
            <input
              id="email-test-to"
              v-model="testTo"
              type="email"
              class="input"
              :class="{ 'input-error': testField }"
              :placeholder="store.settings.replyTo ?? ''"
              data-test="email-test-to"
            />
          </FormField>
        </div>
        <button type="submit" class="btn" :disabled="testing || !store.settings.configured || store.settings.disabled" data-test="email-test-send">
          {{ testing ? t("email.send.sending") : t("email.settings.testSend") }}
        </button>
      </form>
      <div v-if="testError" role="alert" class="alert-error" data-test="email-test-error">
        <p>{{ testError }}</p>
        <ErrorDetail :detail="testDetail" open />
      </div>
    </section>

    <section v-if="store.templates.length" class="card space-y-3">
      <h2 class="text-lg font-semibold">{{ t("email.templates.title") }}</h2>
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("email.templates.help") }}</p>
      <div role="tablist" class="flex gap-1 border-b border-gray-200 dark:border-gray-800">
        <button
          v-for="tpl in store.templates"
          :key="tpl.locale"
          type="button"
          role="tab"
          :aria-selected="active === tpl.locale"
          class="border-b-2 px-3 py-2 text-sm"
          :class="active === tpl.locale ? 'border-blue-600 font-medium' : 'border-transparent text-gray-600 dark:text-gray-400'"
          :data-test="`template-tab-${tpl.locale}`"
          @click="active = tpl.locale"
        >
          {{ t(`locale.names.${tpl.locale}`) }}
        </button>
      </div>
      <!-- v-show keeps unsaved edits of the other language while switching tabs. -->
      <EmailTemplateEditor v-for="tpl in store.templates" v-show="active === tpl.locale" :key="tpl.locale" :template="tpl" />
      <TemplateVariables />
    </section>
  </div>
</template>
