import { describe, expect, it, vi } from "vitest";
import { loadOnce } from "./loadOnce";

function setup() {
  let loaded = false;
  const pending: { resolve: () => void; reject: (e: Error) => void }[] = [];
  const load = vi.fn(
    () =>
      new Promise<void>((resolve, reject) => {
        pending.push({
          resolve: () => {
            loaded = true;
            resolve();
          },
          reject,
        });
      }),
  );
  return { ensure: loadOnce(load, () => loaded), load, pending };
}

describe("loadOnce", () => {
  it("shares the in-flight load between concurrent callers", async () => {
    const { ensure, load, pending } = setup();
    const a = ensure();
    const b = ensure();
    expect(load).toHaveBeenCalledTimes(1);
    pending[0].resolve();
    await expect(Promise.all([a, b])).resolves.toEqual([undefined, undefined]);
  });

  it("does not load again once loaded", async () => {
    const { ensure, load, pending } = setup();
    const first = ensure();
    pending[0].resolve();
    await first;
    await ensure();
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("rejects every waiting caller and retries on the next call after a failure", async () => {
    const { ensure, load, pending } = setup();
    const a = ensure();
    const b = ensure();
    pending[0].reject(new Error("down"));
    await expect(a).rejects.toThrow("down");
    await expect(b).rejects.toThrow("down");
    const retry = ensure();
    expect(load).toHaveBeenCalledTimes(2);
    pending[1].resolve();
    await expect(retry).resolves.toBeUndefined();
  });
});
