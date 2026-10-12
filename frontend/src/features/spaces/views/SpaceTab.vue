<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import FormField from "@/components/form/FormField.vue";
import { useFormSubmit } from "@/composables/useFormSubmit";
import { collectErrors, textRule } from "@/lib/formErrors";
import { useSessionStore } from "@/stores/session";
import { spacesApi } from "../api";
import DeleteSpaceSection from "../components/DeleteSpaceSection.vue";
import RequireMfaSection from "../components/RequireMfaSection.vue";
import StorageUsage from "../components/StorageUsage.vue";
import type { Space } from "../types";

const { t } = useI18n();
const session = useSessionStore();
const space = ref<Space | null>(null);
const name = ref("");
const saved = ref(false);
const { fieldErrors, error, submitting, submit, showError } = useFormSubmit();

onMounted(async () => {
  try {
    space.value = await spacesApi.current();
    name.value = space.value.name;
  } catch (err) {
    showError(err);
  }
});

async function onSubmit() {
  saved.value = false;
  saved.value = await submit(
    () => collectErrors({ name: textRule(name.value, { required: true, max: 200 }) }),
    async () => {
      space.value = await spacesApi.update({ name: name.value.trim() });
      name.value = space.value.name;
      session.renameSpace(space.value.name);
    },
  );
}
</script>

<template>
  <div class="space-y-6">
    <form class="card space-y-4" novalidate @submit.prevent="onSubmit">
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("spaces.settings.intro") }}</p>
      <fieldset class="grid gap-4 sm:grid-cols-2" :disabled="!space">
        <FormField :label="t('spaces.create.name')" for="space-rename" :error="fieldErrors.name">
          <input id="space-rename" v-model="name" maxlength="200" class="input" :class="{ 'input-error': fieldErrors.name }" data-test="space-rename" />
        </FormField>
        <FormField :label="t('spaces.settings.slug')" for="space-url">
          <input id="space-url" :value="space?.url ?? ''" readonly class="input font-mono" />
        </FormField>
      </fieldset>
      <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
      <p v-if="saved" role="status" class="text-sm text-green-700 dark:text-green-400">{{ t("spaces.settings.saved") }}</p>
      <button type="submit" class="btn btn-primary" :disabled="submitting || !space" data-test="space-save">{{ t("common.save") }}</button>
    </form>
    <StorageUsage v-if="space?.limits" :limits="space.limits" />
    <RequireMfaSection v-if="space && session.can('spacePolicy')" :space="space" @updated="space = $event" />
    <DeleteSpaceSection v-if="space && session.can('deleteSpace')" :slug="space.slug" />
  </div>
</template>
