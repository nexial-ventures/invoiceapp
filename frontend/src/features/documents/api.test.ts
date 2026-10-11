import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mockFetch, sentRequest } from "@/test-utils";
import { documentsApi, exchangeRatesApi } from "./api";
import type { ComputeRequest, DocumentInput } from "./types";

const input = { docType: "invoice", direction: "issued", lines: [] } as unknown as DocumentInput;

describe("documents api", () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(() => vi.unstubAllGlobals());

  it("lists with only the filters that are set", async () => {
    const fetch = mockFetch(200, { items: [], total: 0 });
    await documentsApi.list({
      direction: "issued",
      docType: "invoice",
      status: "issued",
      paymentState: undefined,
      overdue: true,
      q: " 2026 ",
      from: "2026-01-01",
      to: "",
      limit: 50,
      offset: 100,
    });
    expect(sentRequest(fetch).url).toBe(
      "/api/documents?direction=issued&docType=invoice&status=issued&from=2026-01-01&overdue=true&q=2026&limit=50&offset=100",
    );
  });

  it("omits overdue=false and an empty q", async () => {
    const fetch = mockFetch(200, { items: [], total: 0 });
    await documentsApi.list({ overdue: false, q: " ", limit: 50, offset: 0 });
    expect(sentRequest(fetch).url).toBe("/api/documents?limit=50&offset=0");
  });

  it("CRUD routes", async () => {
    let fetch = mockFetch(200, {});
    await documentsApi.get("d1");
    expect(sentRequest(fetch)).toMatchObject({ url: "/api/documents/d1", method: "GET" });

    fetch = mockFetch(201, {});
    await documentsApi.create(input);
    expect(sentRequest(fetch)).toEqual({ url: "/api/documents", method: "POST", body: input });

    fetch = mockFetch(200, {});
    await documentsApi.update("d1", input);
    expect(sentRequest(fetch)).toEqual({ url: "/api/documents/d1", method: "PUT", body: input });

    fetch = mockFetch(204);
    await documentsApi.remove("d1");
    expect(sentRequest(fetch)).toMatchObject({ url: "/api/documents/d1", method: "DELETE" });
  });

  it("compute passes the abort signal", async () => {
    const body: ComputeRequest = {
      lines: [],
      vatMode: "standard",
      currency: "CZK",
      exchangeRate: null,
      roundingMode: "none",
      contactId: "c1",
      docType: "invoice",
      locale: "cs",
      documentId: "d1",
    };
    const fetch = mockFetch(200, {});
    const controller = new AbortController();
    await documentsApi.compute(body, controller.signal);
    expect(sentRequest(fetch)).toEqual({ url: "/api/documents/compute", method: "POST", body });
    expect(fetch.mock.calls[0]?.[1]?.signal).toBe(controller.signal);
  });

  it("lifecycle actions", async () => {
    let fetch = mockFetch(200, {});
    await documentsApi.issue("d1");
    expect(sentRequest(fetch)).toMatchObject({ url: "/api/documents/d1/issue", method: "POST" });

    fetch = mockFetch(200, {});
    await documentsApi.cancel("d1", "duplicate");
    expect(sentRequest(fetch)).toEqual({ url: "/api/documents/d1/cancel", method: "POST", body: { reason: "duplicate" } });

    fetch = mockFetch(200, {});
    await documentsApi.cancel("d1", null);
    expect(sentRequest(fetch).body).toEqual({});

    fetch = mockFetch(200, {});
    await documentsApi.markSent("d1");
    expect(sentRequest(fetch)).toEqual({ url: "/api/documents/d1/mark-sent", method: "POST", body: {} });

    fetch = mockFetch(200, {});
    await documentsApi.setMetadata("d1", { categoryId: "k1", customFields: { po: "7" }, internalNote: "call first" });
    expect(sentRequest(fetch)).toEqual({
      url: "/api/documents/d1/metadata",
      method: "PUT",
      body: { categoryId: "k1", customFields: { po: "7" }, internalNote: "call first" },
    });
  });

  it("payments", async () => {
    let fetch = mockFetch(200, []);
    await documentsApi.payments("d1");
    expect(sentRequest(fetch)).toMatchObject({ url: "/api/documents/d1/payments", method: "GET" });

    const payment = { date: "2026-10-08", amount: "100.00", note: null };
    fetch = mockFetch(201, {});
    await documentsApi.addPayment("d1", payment);
    expect(sentRequest(fetch)).toEqual({ url: "/api/documents/d1/payments", method: "POST", body: payment });

    fetch = mockFetch(204);
    await documentsApi.removePayment("d1", "p1");
    expect(sentRequest(fetch)).toMatchObject({ url: "/api/documents/d1/payments/p1", method: "DELETE" });
  });

  it("settle and credit note create drafts", async () => {
    let fetch = mockFetch(201, {});
    await documentsApi.settle("pf1");
    expect(sentRequest(fetch)).toMatchObject({ url: "/api/documents/pf1/settle", method: "POST" });

    fetch = mockFetch(201, {});
    await documentsApi.creditNote("d1", "wrong quantity");
    expect(sentRequest(fetch)).toEqual({ url: "/api/documents/d1/credit-note", method: "POST", body: { correctionReason: "wrong quantity" } });

    fetch = mockFetch(201, {});
    await documentsApi.creditNote("d1", null);
    expect(sentRequest(fetch).body).toEqual({});
  });

  it("proforma payment carries an optional manual rate", async () => {
    const payment = { date: "2026-10-08", amount: "100.00", note: null, exchangeRate: "24.5" };
    const fetch = mockFetch(201, {});
    await documentsApi.addPayment("pf1", payment);
    expect(sentRequest(fetch).body).toEqual(payment);
  });

  it("exchange rate for a date", async () => {
    const fetch = mockFetch(200, { currency: "EUR", date: "2026-10-07", rate: "24.3" });
    await exchangeRatesApi.get("EUR", "2026-10-08");
    expect(sentRequest(fetch).url).toBe("/api/exchange-rates/EUR?date=2026-10-08");
  });
});
