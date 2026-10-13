import { onScopeDispose, shallowRef, watch } from "vue";
import { documentsApi } from "./api";
import type { NumberPreview } from "./types";

export const PREVIEW_DEBOUNCE_MS = 300;

export interface NumberPreviewSource {
  docType: string;
  contactId: string | null;
  issueDate: string;
  /** False while the draft holds a number of its own: nothing to preview. */
  enabled: boolean;
}

/**
 * The number an issue would allocate now (client or global series), refetched (debounced) when the doc
 * type, customer or issue date changes. Expected only: issue allocates the final one. A failure shows no
 * preview — the number field works without it.
 */
export function useNumberPreview(source: () => NumberPreviewSource, delay = PREVIEW_DEBOUNCE_MS) {
  const preview = shallowRef<NumberPreview | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;

  async function load(query: Omit<NumberPreviewSource, "enabled">, mine: number): Promise<void> {
    try {
      const res = await documentsApi.numberPreview(query);
      if (mine === seq) preview.value = res;
    } catch {
      if (mine === seq) preview.value = null;
    }
  }

  watch(
    () => {
      const { docType, contactId, issueDate, enabled } = source();
      return enabled && /^\d{4}-\d{2}-\d{2}$/.test(issueDate) ? JSON.stringify({ docType, contactId, issueDate }) : "";
    },
    (key) => {
      clearTimeout(timer);
      const mine = ++seq;
      if (!key) {
        preview.value = null;
        return;
      }
      const query = JSON.parse(key) as Omit<NumberPreviewSource, "enabled">;
      timer = setTimeout(() => void load(query, mine), delay);
    },
    { immediate: true },
  );

  onScopeDispose(() => {
    clearTimeout(timer);
    seq++;
  });

  return { preview };
}
