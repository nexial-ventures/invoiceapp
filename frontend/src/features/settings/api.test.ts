import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mockFetch, sentRequest as call } from "@/test-utils";
import { company as baseCompany } from "@/features/documents/testData";
import { bankAccountsApi, companyApi, numberSeriesApi, vatRatesApi } from "./api";
import type { BankAccountInput, Company, VatRateInput } from "./types";

const company: Company = { ...baseCompany, name: "Acme", ico: "25596641", vatPayer: false };

describe("settings api", () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(() => vi.unstubAllGlobals());

  it("company get/put", async () => {
    let fetch = mockFetch(200, company);
    await companyApi.get();
    expect(call(fetch)).toEqual({ url: "/api/settings/company", method: "GET", body: undefined });

    fetch = mockFetch(200, company);
    await companyApi.update(company);
    expect(call(fetch)).toEqual({ url: "/api/settings/company", method: "PUT", body: company });
  });

  it("bank accounts CRUD", async () => {
    const input: BankAccountInput = {
      label: "Main",
      currency: "CZK",
      accountNumber: "123456789/0100",
      iban: null,
      bic: null,
      isDefault: true,
    };
    let fetch = mockFetch(200, []);
    await bankAccountsApi.list();
    expect(call(fetch)).toMatchObject({ url: "/api/settings/bank-accounts", method: "GET" });

    fetch = mockFetch(200, { id: "a", ...input });
    await bankAccountsApi.create(input);
    expect(call(fetch)).toEqual({ url: "/api/settings/bank-accounts", method: "POST", body: input });

    fetch = mockFetch(200, { id: "a", ...input });
    await bankAccountsApi.update("a", input);
    expect(call(fetch)).toEqual({ url: "/api/settings/bank-accounts/a", method: "PUT", body: input });

    fetch = mockFetch(204);
    await bankAccountsApi.remove("a");
    expect(call(fetch)).toMatchObject({ url: "/api/settings/bank-accounts/a", method: "DELETE" });
  });

  it("vat rates send the rate as a decimal string", async () => {
    const input: VatRateInput = { rate: "12.5", label: "X", isDefault: false, active: true, position: 3 };
    const fetch = mockFetch(200, { id: "v", ...input });
    await vatRatesApi.update("v", input);
    expect(call(fetch)).toEqual({ url: "/api/settings/vat-rates/v", method: "PUT", body: input });
  });

  it("number series pattern and counter", async () => {
    let fetch = mockFetch(200, []);
    await numberSeriesApi.list();
    expect(call(fetch)).toMatchObject({ url: "/api/settings/number-series", method: "GET" });

    fetch = mockFetch(200, {});
    await numberSeriesApi.updatePattern("credit_note", "D{YYYY}{NNN}");
    expect(call(fetch)).toEqual({
      url: "/api/settings/number-series/credit_note",
      method: "PUT",
      body: { pattern: "D{YYYY}{NNN}" },
    });

    fetch = mockFetch(200, {});
    await numberSeriesApi.setCounter("invoice", 2026, 41);
    expect(call(fetch)).toEqual({
      url: "/api/settings/number-series/invoice/counters/2026",
      method: "PUT",
      body: { lastNumber: 41 },
    });
  });
});
