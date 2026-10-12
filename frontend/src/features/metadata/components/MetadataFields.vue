<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { FieldErrors, Direction } from "@/api/types";
import FormField from "@/components/form/FormField.vue";
import type { Category, CustomField } from "@/features/settings/types";
import { categoryKindOf, categoryOptions, customFieldErrors, type MetadataDraft } from "../metadata";
import CustomFieldsInputs from "./CustomFieldsInputs.vue";

/** Category, custom fields and internal note; `errors` use the wire keys (`categoryId`, `customFields.<key>`, `internalNote`). */
const model = defineModel<MetadataDraft>({ required: true });
const props = defineProps<{
  direction: Direction;
  categories: Category[];
  fieldDefs: CustomField[];
  errors: FieldErrors;
  idPrefix: string;
}>();

const { t } = useI18n();
const options = computed(() => categoryOptions(props.categories, categoryKindOf(props.direction), model.value.categoryId || null));
const cfErrors = computed(() => customFieldErrors(props.errors));
const id = (field: string) => `${props.idPrefix}-${field}`;
</script>

<template>
  <div class="grid gap-4 sm:grid-cols-3">
    <FormField :label="t('metadata.category')" :for="id('categoryId')" :error="errors.categoryId">
      <select :id="id('categoryId')" v-model="model.categoryId" class="input" :class="{ 'input-error': errors.categoryId }">
        <option value="">{{ t("metadata.noCategory") }}</option>
        <option v-for="c in options" :key="c.id" :value="c.id">{{ c.active ? c.name : t("metadata.inactiveCategory", { name: c.name }) }}</option>
      </select>
    </FormField>
    <CustomFieldsInputs v-model="model.customFields" :defs="fieldDefs" :errors="cfErrors" :id-prefix="idPrefix" />
    <FormField class="sm:col-span-3" :label="t('documents.fields.internalNote')" :for="id('internalNote')" :error="errors.internalNote" :hint="t('documents.editor.internalNoteHint')">
      <textarea :id="id('internalNote')" v-model="model.internalNote" rows="2" maxlength="2000" class="input" />
    </FormField>
  </div>
</template>
