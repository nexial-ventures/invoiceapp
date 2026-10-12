import { computed } from "vue";
import type { Direction } from "@/api/types";
import { useCategoriesStore, useCustomFieldsStore } from "@/features/settings/stores";
import { applicableFields } from "./metadata";

/** Categories + the custom field definitions applicable to `direction`, loaded once per session. */
export function useMetadataSettings(direction: () => Direction) {
  const categories = useCategoriesStore();
  const customFields = useCustomFieldsStore();

  async function load(): Promise<void> {
    await Promise.all([categories.ensure(), customFields.ensure()]);
  }

  return {
    categories: computed(() => categories.items),
    fieldDefs: computed(() => applicableFields(customFields.items, direction())),
    load,
  };
}
