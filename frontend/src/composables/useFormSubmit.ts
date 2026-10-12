import { ref } from "vue";
import { useI18n } from "vue-i18n";
import type { FieldErrors } from "@/api/types";
import { fieldErrorsOf } from "@/lib/formErrors";
import { useErrorText } from "./useAction";

/**
 * Submit state for a form: client-side checks first, then the request; a 422
 * from the server replaces the field errors (server is the source of truth).
 */
/** `errorText` overrides how a non-field error reads (e.g. the e-mail limit message). */
export function useFormSubmit(errorText: (err: unknown) => string = useErrorText()) {
  const { t } = useI18n();
  const fieldErrors = ref<FieldErrors>({});
  const error = ref<string | null>(null);
  const submitting = ref(false);

  function showError(err: unknown): void {
    const fields = fieldErrorsOf(err);
    if (fields) {
      fieldErrors.value = fields;
      error.value = t("errors.validation");
      return;
    }
    error.value = errorText(err);
  }

  /** Returns true when the action succeeded. */
  async function submit(validate: () => FieldErrors, action: () => Promise<void>): Promise<boolean> {
    if (submitting.value) return false;
    error.value = null;
    fieldErrors.value = validate();
    if (Object.keys(fieldErrors.value).length > 0) return false;
    submitting.value = true;
    try {
      await action();
      return true;
    } catch (err) {
      showError(err);
      return false;
    } finally {
      submitting.value = false;
    }
  }

  return { fieldErrors, error, submitting, submit, showError };
}
