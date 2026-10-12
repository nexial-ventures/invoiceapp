import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useErrorText } from "@/composables/useAction";
import { fieldErrorsOf, reasonKey } from "@/lib/formErrors";

/**
 * Like `useAction`, but a 422 from a table action (no form fields to point at) shows its reason
 * (`role: too_high | last_owner`) instead of the generic "the form has errors".
 */
export function useMemberAction(errorText: (err: unknown) => string = useErrorText()) {
  const { t } = useI18n();
  const error = ref<string | null>(null);

  async function run(action: () => Promise<unknown>): Promise<boolean> {
    error.value = null;
    try {
      await action();
      return true;
    } catch (err) {
      const reason = Object.values(fieldErrorsOf(err) ?? {})[0];
      error.value = reason ? t(reasonKey(reason)) : errorText(err);
      return false;
    }
  }

  return { error, run };
}
