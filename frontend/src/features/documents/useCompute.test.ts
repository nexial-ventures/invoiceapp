import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectScope, nextTick, ref } from "vue";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises } from "@vue/test-utils";
import type { ComputeRequest } from "./types";
import { useCompute } from "./useCompute";

const body = (currency: string): ComputeRequest => ({
  lines: [],
  vatMode: "standard",
  currency,
  exchangeRate: null,
  roundingMode: "none",
  contactId: null,
  docType: "invoice",
  locale: "cs",
});
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "Content-Type": "application/json" } });

/** fetch stub whose responses the test resolves by hand, in any order. */
function deferredFetch() {
  const pending: { body: ComputeRequest; signal: AbortSignal | null | undefined; resolve: (r: Response) => void }[] = [];
  const fn = vi.fn<typeof fetch>(
    (_url, init) =>
      new Promise<Response>((resolve) => {
        pending.push({ body: JSON.parse(String(init?.body)) as ComputeRequest, signal: init?.signal, resolve });
      }),
  );
  vi.stubGlobal("fetch", fn);
  return { fn, pending };
}

describe("useCompute", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  function setup() {
    const source = ref(body("CZK"));
    const scope = effectScope();
    const state = scope.run(() => useCompute(() => source.value))!;
    return { source, state, scope };
  }

  it("debounces changes into a single request with the latest body", async () => {
    const { fn, pending } = deferredFetch();
    const { source, state, scope } = setup();
    source.value = body("EUR");
    await nextTick();
    source.value = body("USD");
    await nextTick();
    expect(state.pending.value).toBe(true);
    vi.advanceTimersByTime(299);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(1);
    expect(pending[0]?.body.currency).toBe("USD");
    scope.stop();
  });

  it("keeps only the latest response and aborts the stale request", async () => {
    const { pending } = deferredFetch();
    const { source, state, scope } = setup();
    vi.advanceTimersByTime(300); // request #1 (CZK)
    source.value = body("EUR");
    await nextTick();
    vi.advanceTimersByTime(300); // request #2 (EUR)
    expect(pending).toHaveLength(2);
    expect(pending[0]?.signal?.aborted).toBe(true);

    pending[1]?.resolve(json({ lines: [], totals: { payable: "2" } }));
    await flushPromises();
    pending[0]?.resolve(json({ lines: [], totals: { payable: "1" } }));
    await flushPromises();

    expect(state.result.value?.totals.payable).toBe("2");
    expect(state.pending.value).toBe(false);
    scope.stop();
  });

  it("stays pending while a newer edit waits on the debounce", async () => {
    const { pending } = deferredFetch();
    const { source, state, scope } = setup();
    vi.advanceTimersByTime(300); // request #1 in flight
    source.value = body("EUR"); // newer edit, still debouncing
    await nextTick();
    vi.advanceTimersByTime(100);
    pending[0]?.resolve(json({ lines: [], totals: { payable: "1" } }));
    await flushPromises();
    expect(state.result.value?.totals.payable).toBe("1");
    expect(state.pending.value).toBe(true);

    vi.advanceTimersByTime(200); // request #2
    pending[1]?.resolve(json({ lines: [], totals: { payable: "2" } }));
    await flushPromises();
    expect(state.pending.value).toBe(false);
    scope.stop();
  });

  it("exposes 422 field errors and keeps the last good result", async () => {
    const { pending } = deferredFetch();
    const { source, state, scope } = setup();
    vi.advanceTimersByTime(300);
    pending[0]?.resolve(json({ lines: [], totals: { payable: "1" } }));
    await flushPromises();

    source.value = body("EUR");
    await nextTick();
    vi.advanceTimersByTime(300);
    pending[1]?.resolve(json({ code: "validation", fields: { "lines.0.quantity": "invalid" } }, 422));
    await flushPromises();

    expect(state.fieldErrors.value).toEqual({ "lines.0.quantity": "invalid" });
    expect(state.error.value).toBeNull();
    expect(state.result.value?.totals.payable).toBe("1");
    scope.stop();
  });
});
