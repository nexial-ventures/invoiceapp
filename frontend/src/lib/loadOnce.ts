/**
 * A store's `ensure()`: runs `load` unless `isLoaded()`. Concurrent callers share the in-flight
 * promise. A failure is not cached: it rejects every waiting caller and the next call retries.
 * Callers for whom the data is optional ignore the rejection (`ensure().catch(() => {})`).
 */
export function loadOnce(load: () => Promise<void>, isLoaded: () => boolean): () => Promise<void> {
  let inFlight: Promise<void> | null = null;
  return () => {
    if (isLoaded()) return Promise.resolve();
    inFlight ??= load().finally(() => {
      inFlight = null;
    });
    return inFlight;
  };
}
