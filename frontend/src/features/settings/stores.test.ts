import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mockFetchSequence } from "@/test-utils";
import { useBankAccountsStore, useNumberSeriesStore } from "./stores";
import type { BankAccount, NumberSeries } from "./types";

function account(id: string, isDefault: boolean): BankAccount {
  return { id, label: id, currency: "CZK", accountNumber: "1234/0100", iban: null, bic: null, isDefault };
}

describe("settings stores", () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(() => vi.unstubAllGlobals());

  it("reloads the list after save so server-side default flips are reflected", async () => {
    const store = useBankAccountsStore();
    store.items = [account("a", true), account("b", false)];
    const { id: _id, ...input } = account("b", true);
    const fetch = mockFetchSequence(account("b", true), [account("a", false), account("b", true)]);

    await store.save(input, "b");

    expect(fetch.mock.calls.map((c) => [c[0], c[1]?.method ?? "GET"])).toEqual([
      ["/api/settings/bank-accounts/b", "PUT"],
      ["/api/settings/bank-accounts", "GET"],
    ]);
    expect(store.items.map((a) => [a.id, a.isDefault])).toEqual([
      ["a", false],
      ["b", true],
    ]);
  });

  it("reloads the list after delete", async () => {
    const store = useBankAccountsStore();
    const fetch = mockFetchSequence(undefined, [account("a", true)]);
    await store.remove("b");
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(store.items).toHaveLength(1);
  });

  it("ensure loads once for concurrent callers and not again once loaded", async () => {
    const store = useBankAccountsStore();
    const fetch = mockFetchSequence([account("a", true)]);
    await Promise.all([store.ensure(), store.ensure()]);
    await store.ensure();
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(store.items).toHaveLength(1);
  });

  it("number series replaces only the saved doc type", async () => {
    const store = useNumberSeriesStore();
    const s = (docType: NumberSeries["docType"], pattern: string): NumberSeries => ({
      docType,
      pattern,
      counters: [],
      nextNumberPreview: "",
    });
    store.series = [s("invoice", "{YYYY}{NNNN}"), s("proforma", "Z{YYYY}{NNNN}")];
    mockFetchSequence(s("invoice", "F{YY}{NNN}"));

    await store.savePattern("invoice", "F{YY}{NNN}");

    expect(store.series.map((x) => x.pattern)).toEqual(["F{YY}{NNN}", "Z{YYYY}{NNNN}"]);
  });
});
