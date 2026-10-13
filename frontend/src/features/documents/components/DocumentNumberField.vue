<script setup lang="ts">
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import type { DocumentDraft } from "../form";
import { useNumberPreview } from "../useNumberPreview";

/**
 * The number of a native draft: empty = the sequence allocates it at issue (the expected number is
 * previewed); typed = a manual number (fills a gap, fixes a mistake). An unlocked draft starts with its
 * kept number. Only a touched number is sent (`numberTouched`).
 */
const model = defineModel<DocumentDraft>({ required: true });
defineProps<{ error?: string }>();

const { t } = useI18n();
const { preview } = useNumberPreview(() => ({
  docType: model.value.docType,
  contactId: model.value.contactId,
  issueDate: model.value.issueDate,
  enabled: model.value.number.trim() === "",
}));

function onInput() {
  model.value.numberTouched = true;
}

/** Back to the sequence: sent as `null`, the issue allocates afresh. */
function fromSequence() {
  model.value.number = "";
  model.value.numberTouched = true;
}
</script>

<template>
  <div class="space-y-1" data-test="number-field">
    <FormField :label="t('documents.fields.number')" for="doc-number" :error="error">
      <div class="flex flex-wrap gap-2">
        <input
          id="doc-number"
          v-model="model.number"
          maxlength="50"
          class="input max-w-xs font-mono"
          :class="{ 'input-error': error }"
          :placeholder="preview?.number ?? ''"
          autocomplete="off"
          spellcheck="false"
          @input="onInput"
        />
        <button v-if="model.number.trim()" type="button" class="btn" data-test="number-from-sequence" @click="fromSequence">
          {{ t("numbering.document.fromSequence") }}
        </button>
      </div>
    </FormField>
    <template v-if="!error">
      <p v-if="model.number.trim()" class="text-xs text-gray-500 dark:text-gray-400" data-test="number-manual-hint">
        {{ t("numbering.document.manualHint") }}
      </p>
      <i18n-t v-else-if="preview" keypath="numbering.document.willIssue" tag="p" class="text-xs text-gray-500 dark:text-gray-400" data-test="number-preview">
        <template #number>
          <strong class="font-mono text-gray-800 dark:text-gray-200">{{ preview.number }}</strong>
        </template>
        <template #source>{{ t(`numbering.document.source.${preview.source}`) }}</template>
      </i18n-t>
      <p v-else class="text-xs text-gray-500 dark:text-gray-400">{{ t("numbering.document.emptyHint") }}</p>
    </template>
  </div>
</template>
