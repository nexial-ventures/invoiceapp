import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { calls, mockFetchRoutes, reply } from "@/test-utils";
import { mountView } from "@/testMount";
import type { Overview } from "../types";
import OverviewView from "./OverviewView.vue";

const zero = { count: 0, amount: "0.00", base: "0.00", vat: "0.00", total: "0.00" };
const noneOpen = { count: 0, amount: "0.00", overdueCount: 0, overdueAmount: "0.00", topOverdue: [], advances: { count: 0, amount: "0.00" } };

function fixture(vatPayer = true): Overview {
  return {
    from: "2026-01-01",
    to: "2026-01-31",
    today: "2026-10-13",
    vatPayer,
    income: { count: 2, amount: "1500.00", base: "1500.00", vat: "270.00", total: "1770.00" },
    expense: { ...zero, count: 1, amount: "1112.00" },
    difference: "388.00",
    vat: {
      output: [
        { rate: "21", base: "1000.00", vat: "210.00" },
        { rate: "12", base: "500.00", vat: "60.00" },
      ],
      input: [],
      outputVat: "270.00",
      inputVat: "0.00",
      result: "-15.00",
      reverseChargeIssued: "700.00",
      reverseChargeReceived: "0.00",
      nonDeductible: { rate: "0", base: "100.00", vat: "12.00" },
    },
    receivables: {
      ...noneOpen,
      count: 1,
      amount: "1210.00",
      overdueCount: 1,
      overdueAmount: "1210.00",
      topOverdue: [{ id: "d1", docType: "invoice", number: "20260001", counterparty: "Odběratel a.s.", dueDate: "2026-01-24", remaining: "1210.00" }],
      advances: { count: 1, amount: "121.00" },
    },
    payables: noneOpen,
    months: [
      { month: "2025-12", income: "300.00", expense: "0.00", inPeriod: false },
      { month: "2026-01", income: "1500.00", expense: "1112.00", inPeriod: true },
    ],
    categories: {
      income: [
        { id: null, name: null, count: 1, amount: "1000.00" },
        { id: "c1", name: "Vývoj", count: 1, amount: "500.00" },
      ],
      expense: [],
    },
  };
}

describe("OverviewView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 9, 13, 12));
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  it("shows the period of the URL: tiles, chart, VAT, receivables and categories", async () => {
    const fetch = mockFetchRoutes({ "GET /api/overview": () => fixture() });
    const { w } = await mountView(OverviewView, { path: "/?period=month&from=2026-01-15", role: "accountant" });
    expect(calls(fetch)).toEqual(["GET /api/overview?from=2026-01-01&to=2026-01-31"]);
    expect(w.find('[data-test="period-label"]').text()).toBe("January 2026");
    expect(w.find('[data-test="tile-income"] [data-test="tile-amount"]').text()).toContain("1,500.00");
    expect(w.find('[data-test="tile-difference"]').text()).toContain("388.00");
    expect(w.findAll('[data-test="chart-month"]').map((g) => g.attributes("data-in-period"))).toEqual(["false", "true"]);
    expect(w.find('[data-test="chart-trailing"]').exists()).toBe(true);
    expect(w.findAll('[data-test="vat-output"] [data-test="vat-row"]')).toHaveLength(2);
    expect(w.find('[data-test="vat-result"]').text()).toContain("VAT refund");
    expect(w.find('[data-test="vat-result"]').text()).not.toContain("-");
    expect(w.find('[data-test="vat-rc-issued"]').exists()).toBe(true);
    expect(w.find('[data-test="vat-rc-received"]').exists()).toBe(false);
    expect(w.find('[data-test="vat-non-deductible"]').exists()).toBe(true);
    const top = w.find('[data-test="open-issued"] [data-test="open-top"]');
    expect(top.text()).toContain("Odběratel a.s.");
    expect(top.find("a").attributes("href")).toBe("/invoices/d1");
    expect(w.find('[data-test="open-issued"] [data-test="open-advances"]').text()).toContain("121.00");
    const cats = w.findAll('[data-test="categories-income"] [data-test="category"]').map((c) => c.text());
    expect(cats[0]).toContain("Uncategorized");
    expect(cats[1]).toContain("Vývoj");
    expect(w.find('[data-test="categories-expense"]').text()).toContain("No documents");
  });

  it("hides the VAT block of a non-payer", async () => {
    mockFetchRoutes({ "GET /api/overview": () => fixture(false) });
    const { w } = await mountView(OverviewView, { path: "/", role: "member" });
    expect(w.find('[data-test="vat-block"]').exists()).toBe(false);
    expect(w.find('[data-test="tile-income"]').text()).toContain("(total)");
  });

  it("defaults to the current month and keeps the chosen period in the URL", async () => {
    const fetch = mockFetchRoutes({ "GET /api/overview": () => fixture() });
    const { w, router } = await mountView(OverviewView, { path: "/", role: "accountant" });
    await w.find('[data-test="period-next"]').trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.query).toEqual({ period: "month", from: "2026-11-01", to: "2026-11-30" });
    await w.find('[data-test="period-quarter"]').trigger("click");
    await flushPromises();
    await w.find('[data-test="period-prev"]').trigger("click");
    await flushPromises();
    await w.find('[data-test="period-custom"]').trigger("click");
    await flushPromises();
    await w.find('[data-test="period-from"]').setValue("2026-07-15");
    await w.find('[data-test="period-to"]').setValue("2026-07-01");
    expect(w.find('[data-test="period-apply"]').attributes("disabled")).toBeDefined();
    await w.find('[data-test="period-to"]').setValue("2026-08-20");
    await w.find('[data-test="period-custom-form"]').trigger("submit");
    await flushPromises();
    expect(calls(fetch)).toEqual([
      "GET /api/overview?from=2026-10-01&to=2026-10-31",
      "GET /api/overview?from=2026-11-01&to=2026-11-30",
      "GET /api/overview?from=2026-10-01&to=2026-12-31",
      "GET /api/overview?from=2026-07-01&to=2026-09-30",
      "GET /api/overview?from=2026-07-15&to=2026-08-20",
    ]);
    expect(router.currentRoute.value.query.period).toBe("custom");
  });

  it("shows a failure", async () => {
    mockFetchRoutes({ "GET /api/overview": () => reply(500, { code: "internal" }) });
    const { w } = await mountView(OverviewView, { path: "/", role: "accountant" });
    expect(w.find('[data-test="overview-error"]').exists()).toBe(true);
    expect(w.find('[data-test="overview"]').exists()).toBe(false);
  });
});
