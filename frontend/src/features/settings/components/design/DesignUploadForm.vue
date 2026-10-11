<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import ErrorDetail from "@/components/ErrorDetail.vue";
import FormField from "@/components/form/FormField.vue";
import { useErrorText } from "@/composables/useAction";
import { errorDetailOf, fieldErrorsOf } from "@/lib/formErrors";
import { useDesignStore } from "../../stores";

/** Uploads one file into the draft under an editable target path (default: the file name). */
const { t } = useI18n();
const store = useDesignStore();
const errorText = useErrorText();

const input = ref<HTMLInputElement | null>(null);
const file = ref<File | null>(null);
const path = ref("");
// The path this form filled in itself: a newly picked file may overwrite it, a typed (or "replace") path not.
// It is the lowercased file name (upload paths are lowercase); "replace" keeps the exact live path instead.
let autoPath = "";
const busy = ref(false);
const error = ref<string | null>(null);
const detail = ref<string | null>(null);

const ready = computed(() => file.value !== null && path.value.trim() !== "" && !busy.value);

function onFile(event: Event) {
  file.value = (event.target as HTMLInputElement).files?.[0] ?? null;
  if (!file.value || (path.value !== "" && path.value !== autoPath)) return;
  autoPath = file.value.name.toLowerCase();
  path.value = autoPath;
}

/** "Replace" of a live file: fixes the target path and opens the file picker. */
function pick(target: string) {
  path.value = target;
  autoPath = "";
  input.value?.click();
}

/** 422 `validation` gets the specific message; `quota_exceeded` keeps its `detail` (used / limit). */
function showError(err: unknown) {
  const fields = fieldErrorsOf(err);
  if (fields?.path) error.value = t("design.draft.pathInvalid");
  else if (fields?.file === "too_large") error.value = t("design.draft.tooLarge");
  else error.value = errorText(err);
  detail.value = errorDetailOf(err);
}

async function upload() {
  const chosen = file.value;
  if (!chosen || !ready.value) return;
  busy.value = true;
  error.value = detail.value = null;
  try {
    await store.upload(path.value.trim(), chosen);
    file.value = null;
    path.value = autoPath = "";
    if (input.value) input.value.value = "";
  } catch (err) {
    showError(err);
  } finally {
    busy.value = false;
  }
}

defineExpose({ pick });
</script>

<template>
  <form class="space-y-3" data-test="design-upload" @submit.prevent="upload">
    <div class="grid gap-4 sm:grid-cols-2">
      <FormField :label="t('design.draft.file')" for="design-upload-file">
        <input id="design-upload-file" ref="input" type="file" class="input" data-test="design-upload-file" @change="onFile" />
      </FormField>
      <FormField :label="t('design.draft.path')" for="design-upload-path" :hint="t('design.draft.pathHint')">
        <input
          id="design-upload-path"
          v-model="path"
          class="input font-mono"
          autocomplete="off"
          spellcheck="false"
          data-test="design-upload-path"
        />
      </FormField>
    </div>
    <button type="submit" class="btn btn-primary" :disabled="!ready" data-test="design-upload-submit">
      {{ busy ? t("design.draft.uploading") : t("design.draft.upload") }}
    </button>
    <div v-if="error" role="alert" class="alert-error" data-test="design-upload-error">
      <p>{{ error }}</p>
      <ErrorDetail :detail="detail" open />
    </div>
  </form>
</template>
